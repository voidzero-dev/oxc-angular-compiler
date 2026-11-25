//! Legacy animation event names (`(@trigger.phase)`).

use super::chars::is_js_whitespace;

/// Splits a legacy animation event name (without its `@`) into the trigger name
/// and the phase, like Angular's `parseLegacyAnimationEventName`.
///
/// The name is split at the first `.`. When there is one, both parts are
/// trimmed (`splitAtPeriod`) and the phase is lowercased. When there is none,
/// the name is kept as is and there is no phase.
///
/// - `fade.done`  -> (`fade`, `Some("done")`)
/// - `fade.DONE`  -> (`fade`, `Some("done")`)
/// - `fade.`      -> (`fade`, `Some("")`)
/// - `fade`       -> (`fade`, `None`)
///
/// Reference: binding_parser.ts (`parseLegacyAnimationEventName`), util.ts (`splitAtPeriod`)
pub fn split_legacy_animation_event_name(name: &str) -> (&str, Option<String>) {
    match name.split_once('.') {
        Some((trigger, phase)) => (
            trigger.trim_matches(is_js_whitespace),
            Some(phase.trim_matches(is_js_whitespace).to_lowercase()),
        ),
        None => (name, None),
    }
}

/// The errors Angular reports for a legacy animation event with the trigger
/// name `name` and the phase `phase` (as split by
/// [`split_legacy_animation_event_name`]), in Angular's order.
///
/// - `(@.done)`     -> the trigger name is missing
/// - `(@fade.end)`  -> `end` is not a supported phase
/// - `(@fade)`, `(@fade.)` -> the phase is missing
///
/// Reference: binding_parser.ts (`_parseLegacyAnimationEvent`)
pub fn legacy_animation_event_errors(name: &str, phase: Option<&str>) -> Vec<String> {
    let mut errors = Vec::new();
    if name.is_empty() {
        errors.push("Animation event name is missing in binding".to_string());
    }
    match phase {
        Some(phase) if !phase.is_empty() => {
            if phase != "start" && phase != "done" {
                errors.push(format!(
                    "The provided animation output phase value \"{phase}\" for \"@{name}\" is not \
                     supported (use start or done)"
                ));
            }
        }
        _ => errors.push(format!(
            "The animation trigger output event (@{name}) is missing its phase value name (start \
             or done are currently supported)"
        )),
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn errors(raw: &str) -> Vec<String> {
        let (name, phase) = split_legacy_animation_event_name(raw);
        legacy_animation_event_errors(name, phase.as_deref())
    }

    #[test]
    fn splits_like_angular() {
        assert_eq!(split_legacy_animation_event_name("fade.done"), ("fade", Some("done".into())));
        assert_eq!(split_legacy_animation_event_name("fade.DONE"), ("fade", Some("done".into())));
        assert_eq!(
            split_legacy_animation_event_name(" fade . Start "),
            ("fade", Some("start".into()))
        );
        assert_eq!(
            split_legacy_animation_event_name("fade\u{FEFF}.done"),
            ("fade", Some("done".into()))
        );
        assert_eq!(
            split_legacy_animation_event_name("fade.start.x"),
            ("fade", Some("start.x".into()))
        );
        assert_eq!(split_legacy_animation_event_name("fade."), ("fade", Some(String::new())));
        // No `.`: the name is not trimmed.
        assert_eq!(split_legacy_animation_event_name("fade "), ("fade ", None));
    }

    #[test]
    fn valid_phases_have_no_errors() {
        assert!(errors("fade.done").is_empty());
        assert!(errors("fade.start").is_empty());
        assert!(errors("fade.DONE").is_empty());
    }

    #[test]
    fn reports_angular_errors() {
        assert_eq!(
            errors("fade"),
            [
                "The animation trigger output event (@fade) is missing its phase value name (start or done are currently supported)"
            ]
        );
        assert_eq!(
            errors("fade."),
            [
                "The animation trigger output event (@fade) is missing its phase value name (start or done are currently supported)"
            ]
        );
        assert_eq!(
            errors("fade.end"),
            [
                "The provided animation output phase value \"end\" for \"@fade\" is not supported (use start or done)"
            ]
        );
        assert_eq!(
            errors("fade.start.extra"),
            [
                "The provided animation output phase value \"start.extra\" for \"@fade\" is not supported (use start or done)"
            ]
        );
        assert_eq!(errors(".done"), ["Animation event name is missing in binding"]);
        assert_eq!(
            errors(""),
            [
                "Animation event name is missing in binding",
                "The animation trigger output event (@) is missing its phase value name (start or done are currently supported)",
            ]
        );
    }
}
