//! `SCORE64_REQUIRE`: unset or naming other tools, a missing tool is a visible skip; naming it, a failure.

use super::*;

const WHY: &str = "needs gt2reloc on PATH and the score64-emu helper";

#[test]
fn unset_or_empty_skips_with_the_reason_as_today() {
    for require in [None, Some(""), Some(" , ")] {
        assert_eq!(
            verdict(require, "t", &[Tool::Gt2reloc, Tool::Helper], WHY),
            Ok(format!("SKIPPED t: {WHY}")),
            "{require:?}"
        );
    }
}

#[test]
fn a_named_tool_that_is_missing_fails_the_test() {
    for require in ["helper", "gt2reloc,helper", " gt2reloc , helper "] {
        let e = verdict(Some(require), "t", &[Tool::Helper], WHY).unwrap_err();
        assert!(
            e.contains("t:")
                && e.contains("SCORE64_REQUIRE")
                && e.contains("helper")
                && e.contains(WHY),
            "{require:?}: {e}"
        );
    }
    // Of two missing tools, naming either one is enough.
    let e = verdict(Some("gt2reloc"), "t", &[Tool::Gt2reloc, Tool::Helper], WHY).unwrap_err();
    assert!(e.contains("gt2reloc"), "{e}");
}

#[test]
fn a_missing_tool_that_isnt_named_still_skips() {
    // CI requires the helper; the pack tests then still skip visibly where gt2reloc is missing.
    assert_eq!(
        verdict(Some("helper"), "t", &[Tool::Gt2reloc], WHY),
        Ok(format!("SKIPPED t: {WHY}"))
    );
}

#[test]
fn an_unknown_name_is_a_failure_not_a_silent_skip() {
    let e = verdict(Some("helper,gt2relco"), "t", &[Tool::Gt2reloc], WHY).unwrap_err();
    assert!(
        e.contains("gt2relco") && e.contains("helper, gt2reloc"),
        "{e}"
    );
}

#[test]
fn missing_lists_the_tools_not_found() {
    assert_eq!(
        missing([(Tool::Gt2reloc, false), (Tool::Helper, true)]),
        [Tool::Gt2reloc]
    );
    assert_eq!(missing([(Tool::Gt2reloc, true), (Tool::Helper, true)]), []);
}
