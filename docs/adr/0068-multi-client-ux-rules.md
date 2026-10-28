# ADR-0068: Multi-client UX rules

- **Status:** Accepted
- **Date:** 2026-10-28
- **Phase:** P9.4
- **Related:** ADR-0065, ADR-0066, ADR-0067, ADR-0008, ADR-0039, ADR-0040, ADR-0051, REQ-SEC-001,
  REQ-SEC-002, REQ-VIS-005

## Context

The service now knows who is calling (ADR-0065), decides what each role may do and who may change
the instrument (ADR-0066), and records it all (ADR-0067). Several people now use one instrument
at once: an operator at the microscope, colleagues watching from their desks, an admin. The web
app has to:

- ask for a login only where the service enforces one;
- keep each person from tripping over another;
- say why something can't be done before the server refuses it.

## Decision

- **A login only where the server asks.** `GET /api/siq/auth/me`'s 401 says whether the service
  enforces (`"enforced"`).
  - Without `[auth] enforce`, nothing is asked or gated.
  - With it, a login page stands in front of the shell. The page asked for stays in the URL, and
    shows once logged in.
  - A 401 anywhere ends the session, from a Thing's request or the connection's, and the login
    asks again.
  - Until the server has answered, nothing is gated. The shell waits for that answer anyway.
  - `config/sim.toml` now enforces.
- **The desktop logs in by itself**, as the local console. The shell's `local_secret` command
  works like this:
  - It asks the service where the secret is. `GET /api/siq/auth/local` answers to this computer
    only.
  - It reads the file only if the file is called `local-login`, and gives back only its 64
    hexadecimal digits. A page can't make it read another file.
  - The web app sends the secret to `POST /api/siq/auth/local`.
  - The service makes the local user, `[auth] local_user`, at start if it is missing: an operator
    with a random password that no one is told.
- **Disabled, with the reason, never hidden.** Each control says what it needs, and is disabled
  with the reason while the session lacks it:

  | Need      | Who may                                | Such as                                |
  | --------- | -------------------------------------- | -------------------------------------- |
  | `process` | an operator or an admin                | jobs, exports, Stitch, STOP and Esc    |
  | `operate` | an operator or an admin with the token | the camera, the stage, a calibration   |
  | `admin`   | an admin                               | the users, the audit, deleting data    |

  The reasons are:

  - "Log in first."
  - "View only: your role is viewer."
  - "Take control to change the instrument."
  - "Only an admin can do this."

  A pane of controls is a `fieldset` (`ControlPane need`, `GatedFields`), with the reason above
  it. A button on its own, such as a dataset's Delete, carries the reason as its title. The server
  still decides (ADR-0066); the web app only explains in advance. STOP stays enabled for anyone
  who may stop, whatever is moving (ADR-0039, amended).
- **The control token's bar**, above every page:
  - the holder and since when ("you see what they do");
  - Take control for an operator while no one has it, and Release for the holder;
  - Take over for an admin or the local console;
  - who is logged in, and Log out.

  The holder's session beats every 10 s, a third of the server's 30 s. Seeing no one's name or
  another's on the token means this session lost it; a holder not yet known doesn't.
- **The first-run calibration prompt** opens by itself only for a session that may change the
  instrument (ADR-0051, amended). That is an operator holding the token, and it opens when one
  takes control.
- **The Admin page:**
  - **Users:** each with a role to choose. A new password is asked for in a dialog, not in the
    clear. Remove asks first. A new password and a removal end the user's sessions; a new role
    applies to them at once.
  - **The audit trail:** filtered by who (at Enter) and by what, 25 entries a page, newest first.
  - **`AppDataTable`** (ADR-0008): TanStack Table's headless core, styled with the tokens.
    Headers sort, with `aria-sort`; a cell comes from its column or from a slot.
- **A question asked from a dialog stays on top.** An alert dialog has its own layer
  (`--z-alert`), above dialogs, whichever was mounted first. The shell now mounts after the
  login, and so after the overlay provider's own question.
- **Scripts log in too:** `SIQ_USER` and `SIQ_PASSWORD` (`examples/python/siq_session.py`). A
  script that changes the instrument takes the token, and keeps it with a heartbeat.

## Alternatives considered

- **Hiding what a role can't use:** people don't learn why it's missing, or whom to ask. Pages
  would change shape by role, and guides and screenshots wouldn't match.
- **A `/login` route with a router guard:** the page asked for would have to be carried through a
  redirect. A login in front of the shell keeps the URL as it is.
- **Taking the token at the first change:** two operators would take it from each other without
  meaning to. Asking for it is one click, and the bar says who has it.
- **A banner on each page for another holder:** one bar above every page says it the same way
  everywhere, and has the buttons.
- **A table of our own, or PrimeVue's:** sorting, paging and column definitions were the work, and
  PrimeVue was set aside in ADR-0008. TanStack Table is headless and MIT, so the tokens style it.

## Consequences

- **A new control that changes something** should say what it needs, through `ControlPane need`,
  `GatedFields` or `useGate`. Otherwise it fails with the server's 403, which shows as a toast.
- **Keys and gestures on the image aren't gated:** the arrows, the wheel, double-click to move.
  Without the token, the server refuses them, and the toast says why.
- **The simulator asks for a login:** add a user first (`spectrumiq-server user add`). Scripts
  need `SIQ_USER`.
- **The development pages** (`/#/dev/…`) bring their own fakes and ask for no login.
  `/#/dev/admin` shows each role's view.

## Verification

- `web/tests/unit/auth/session.spec.ts`:
  - a login asked for only where the server enforces;
  - a wrong password refused, a logout, and a 401 ending the session;
  - the desktop logged in by the local secret;
  - each reason, by role and token.
- `web/tests/unit/control/TokenBar.spec.ts`:
  - View only for a viewer, its controls disabled with the reason;
  - Take control for an operator, then Release, the controls following;
  - another's name, with Take over for an admin and not for an operator.
- `web/tests/unit/ui/AppDataTable.spec.ts`: the table's contract, with its cells, sorting and the
  empty text.
- `web/tests/unit/admin/AdminPage.spec.ts`:
  - the users, and the audit filtered by who;
  - a user added;
  - a new password asked for in a dialog;
  - a removal only once asked;
  - the page refused to others.
- `web/tests/unit/calibration/wizard.spec.ts`: no first-run prompt until the session takes
  control. `web/tests/unit/control/control.spec.ts`: STOP and Esc disabled for a viewer.
- `crates/siq-server/tests/authz.rs`:
  - the users managed by an admin, with the sessions following;
  - the secret's file named to this computer only;
  - the local user made by the service.
- `apps/spectrumiq-desktop`: the secret read only from `local-login`
  (`the_local_secret_is_read_from_the_file_the_service_names_and_no_other`).
- By hand, in Edge, three users against the simulator:
  - a viewer: disabled controls, no wizard, and the reasons shown;
  - an operator: the wizard once in control, and the controls on;
  - the viewer and an admin: seeing who has control, with Take over offered to the admin only;
  - the admin: managing the users and reading the audit.
