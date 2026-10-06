//! `Severity::Info`: valid and without effect on the result (SREP 18); never counted as a warning.

use sr_model::{Diagnostic, Loc, Report, Severity};

fn report() -> Report {
    let l = Loc::default();
    Report {
        diagnostics: vec![
            Diagnostic::error("C1", "e", l, "/scene"),
            Diagnostic::warning("E20", "w", l, "/scene"),
            Diagnostic::info("E19", "i", l, "/scene"),
            Diagnostic::info("E19", "i2", l, "/scene"),
        ],
    }
}

#[test]
fn information_is_counted_apart_from_warnings() {
    let r = report();
    assert_eq!((r.error_count(), r.warning_count(), r.info_count()), (1, 1, 2));
    assert!(r.to_string().ends_with("1 error(s), 1 warning(s), 2 info"), "{r}");
    let only_info = Report { diagnostics: vec![Diagnostic::info("E19", "i", Loc::default(), "/scene")] };
    assert!(only_info.to_string().ends_with("0 error(s), 0 warning(s), 1 info"), "{only_info}");
    assert!(!only_info.has_errors());
}

#[test]
fn information_serialises_and_displays_as_info() {
    let d = Diagnostic::info("E19", "no effect", Loc::default(), "/scene");
    assert_eq!(d.severity, Severity::Info);
    assert!(d.is_info() && !d.is_error());
    assert_eq!(serde_json::to_value(&d).unwrap()["severity"], "info");
    assert!(d.to_string().starts_with("info[E19]"), "{d}");
}
