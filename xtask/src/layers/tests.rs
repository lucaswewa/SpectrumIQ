use super::*;

// REQ-NFR-007
#[test]
fn crates_depend_on_their_layer_or_below() {
    assert!(Layer::Engines.may_depend_on(Layer::Engines));
    assert!(Layer::Engines.may_depend_on(Layer::Devices));
    assert!(Layer::Service.may_depend_on(Layer::Foundation));
    assert!(!Layer::Core.may_depend_on(Layer::Devices));
    assert!(!Layer::Foundation.may_depend_on(Layer::Core));
}

// REQ-NFR-007
#[test]
fn processing_never_depends_on_devices() {
    assert!(!Layer::Processing.may_depend_on(Layer::Devices));
    assert!(Layer::Processing.may_depend_on(Layer::Processing));
    assert!(Layer::Processing.may_depend_on(Layer::Core));
}

// REQ-NFR-007
#[test]
fn apps_and_tooling_use_anything_and_nothing_uses_them() {
    for layer in [
        Layer::Foundation,
        Layer::Service,
        Layer::App,
        Layer::Tooling,
    ] {
        assert!(Layer::App.may_depend_on(layer));
        assert!(Layer::Tooling.may_depend_on(layer));
    }
    assert!(!Layer::Service.may_depend_on(Layer::App));
    assert!(!Layer::Engines.may_depend_on(Layer::Tooling));
}

#[test]
fn the_table_names_each_crate_once() {
    let mut names: Vec<&str> = TABLE.iter().map(|(name, _)| *name).collect();
    names.sort_unstable();
    let before = names.len();
    names.dedup();
    assert_eq!(names.len(), before);
}
