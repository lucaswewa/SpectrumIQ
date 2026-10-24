//! Dialects: templates that render and match back, the example dialect, and the problems a
//! broken dialect reports.

use proptest::prelude::*;
use siq_ascii::{Dialect, Done, Kind, Template, Value, Values, lengths};

fn template(source: &str) -> Template {
    Template::parse(source).unwrap()
}

proptest! {
    #[test]
    fn a_length_renders_and_matches_back_within_its_precision(
        um in -1.0e6f64..1.0e6,
        step in prop::sample::select(vec![0.01, 0.05, 0.1, 1.0, 2.5]),
        decimals in 0u8..=6,
    ) {
        let cases = [
            ("MA {target:steps}".to_owned(), step / 2.0),
            (format!("MA {{target:um.{decimals}}}"), 0.5 * 10f64.powi(-i32::from(decimals))),
            (format!("MA {{target:mm.{decimals}}}"), 500.0 * 10f64.powi(-i32::from(decimals))),
        ];
        for (source, tolerance) in cases {
            let template = template(&source);
            let line = template.render(&lengths([("target", um)]), step);
            let values = template.matches(&line, step).unwrap();
            let Some(Value::Length(back)) = values.get("target") else { panic!("{line}") };
            prop_assert!((back - um).abs() <= tolerance * (1.0 + 1e-9) + 1e-9, "{source}: {line} → {back}");
        }
    }

    #[test]
    fn numbers_flags_and_text_match_back_exactly(
        code in any::<i64>(),
        flag in any::<bool>(),
        text in "[A-Za-z0-9.-][ -~]{0,20}",
    ) {
        // Regex metacharacters in the literal text match only themselves.
        let template = template("E? {code:int} [{flag:flag}] (*) {text:text}");
        let values = Values::from([
            ("code".to_owned(), Value::Int(code)),
            ("flag".to_owned(), Value::Flag(flag)),
            ("text".to_owned(), Value::Text(text.clone())),
        ]);
        let line = template.render(&values, 1.0);
        prop_assert_eq!(template.matches(&line, 1.0), Some(values));
        prop_assert_eq!(template.matches(&line.replacen('?', "", 1), 1.0), None);
    }
}

#[test]
fn a_template_s_fields_are_named_with_their_kinds() {
    let status = template("ST {position:um.2} {moving:flag} {homed:flag}");
    let fields: Vec<_> = status.fields().collect();
    let expected = [
        ("position", Kind::Um(2)),
        ("moving", Kind::Flag),
        ("homed", Kind::Flag),
    ];
    assert_eq!(fields, expected);
    assert_eq!(
        status.matches("ST 12.50 1 0 ", 1.0),
        None,
        "anchored at both ends"
    );
    assert_eq!(status.matches("XST 12.50 1 0", 1.0), None);
    for (source, problem) in [
        ("MA {target}", "has no kind"),
        ("MA {target:inches}", "isn't a kind of field"),
        ("MA {target:um.12}", "isn't a kind of field"),
        ("MA {Target:steps}", "isn't a field's name"),
        ("MA {target:steps} {target:steps}", "is there twice"),
        ("MA {target:steps", "without its `}`"),
        ("MA target:steps}", "without its `{`"),
    ] {
        let error = Template::parse(source).unwrap_err();
        assert!(error.contains(problem), "{source}: {error}");
    }
}

#[test]
fn the_example_dialect_is_valid_and_is_the_file_in_config() {
    let example = Dialect::example();
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/dialects/example-focus.toml"
    );
    let file = Dialect::load(std::path::Path::new(path)).unwrap();
    assert_eq!(
        (file.name.as_str(), file.step),
        (example.name.as_str(), example.step)
    );
    assert_eq!(example.motion.done, Done::Poll);
    assert!(example.reports("moving") && example.reports("homed"));
    let framing = example.framing();
    assert_eq!((framing.write.as_str(), framing.read), ("\r", b'\r'));
    let error = example.error("ERR 7 too hot").unwrap();
    assert_eq!(
        error.to_string(),
        "device fault: Example focus drive error 7: too hot"
    );
}

const BROKEN: &str = r#"
name = "broken"
step = 0
terminators = { write = "", read = "\r\n" }
timeouts = { reply = 0 }

[motion]
travel = [100.0, 0.0]
speed = -1.0
poll_hz = 500.0
overshoot = -2.0

[replies]
ok = "OK {why:text}"
errors = ["ERR {code:text}"]

[commands]
init = ["ECHO {on:flag}"]
identify = { send = "ID?", reply = "ID {model:steps} {colour:text}" }
move = { send = "MA {to:steps}" }
status = { send = "ST?", reply = "ST {homed:flag}" }
stop = { send = "HALT {now:flag}" }
jog = { send = "JOG", reply = "JOGGING {velocity:steps}" }
"#;

#[test]
fn a_broken_dialect_reports_every_problem_with_its_path() {
    let error = Dialect::parse(BROKEN, "broken.toml").unwrap_err();
    assert_eq!(error.origin, "broken.toml");
    let expected = [
        "step: must be positive",
        "terminators.write: can't be empty",
        "terminators.read: must be one ASCII character",
        "motion.travel: must be [min, max], with min below max",
        "motion.speed: must be positive",
        "motion.poll_hz: must be above 0 and at most 100 Hz",
        "motion.overshoot: must be 0 or more",
        "timeouts.reply: must be positive",
        "replies.ok: has no field {why}",
        "replies.errors[0]: {code} must be a number, not text",
        "commands.init[0]: has no field {on}",
        "commands.identify.reply: {model} must be text, not a length",
        "commands.identify.reply: has no field {colour}",
        "commands.move.send: has no field {to}",
        "commands.move.send: needs a field {target}",
        "commands.status.reply: needs a field {position}",
        "commands.stop.send: has no field {now}",
        "commands.jog.send: needs a field {velocity}",
        "commands.jog.reply: has no field {velocity}",
        "commands.status.reply: needs {moving:flag} when motion.done is \"poll\"",
    ];
    assert_eq!(error.problems, expected);

    // Syntax, unknown keys and bad templates are TOML errors, with their line.
    let kindless = BROKEN.replace("MA {to:steps}", "MA {to}");
    for (text, problem) in [
        ("name = ", "line 1"),
        (
            "name = \"x\"\nstep = 1\nspeling = 2",
            "unknown field `speling`",
        ),
        (kindless.as_str(), "has no kind"),
    ] {
        let error = Dialect::parse(text, "bad.toml").unwrap_err();
        assert!(error.to_string().contains(problem), "{error}");
    }

    // A completion reply is needed to wait for one.
    let reply = Dialect::EXAMPLE.replace("done = \"poll\"", "done = \"reply\"");
    let reply = reply.replace("complete = \"DONE\"", "");
    let error = Dialect::parse(&reply, "reply.toml").unwrap_err();
    let message = "replies.complete: must be given when motion.done is \"reply\"";
    assert_eq!(error.problems, [message]);
}
