//! OWS durations, as `wait`, `timeout` and retry delays write them: an ISO 8601
//! duration such as `PT1M30S`, or an object of `days`, `hours`, `minutes`,
//! `seconds` and `milliseconds`. Durations longer than [`MAX`] are rejected,
//! as are numbers that aren't plain decimals, such as `inf` or `1e30`.

use std::time::Duration;

use serde_json::Value;

use crate::ows;

/// The longest duration marche accepts: 100 years of 365 days.
pub const MAX: Duration = Duration::from_secs(100 * 365 * 86_400);

pub fn parse(value: &Value) -> Result<Duration, ows::Error> {
    match value {
        Value::String(text) => iso8601(text).ok_or_else(|| invalid(value)),
        Value::Object(fields) => {
            let mut millis = 0.0;
            for (unit, amount) in fields {
                let scale = match unit.as_str() {
                    "days" => 86_400_000.0,
                    "hours" => 3_600_000.0,
                    "minutes" => 60_000.0,
                    "seconds" => 1_000.0,
                    "milliseconds" => 1.0,
                    _ => return Err(invalid(value)),
                };
                let amount = amount
                    .as_f64()
                    .filter(|n| *n >= 0.0)
                    .ok_or_else(|| invalid(value))?;
                millis += amount * scale;
            }
            bounded(millis / 1_000.0).ok_or_else(|| invalid(value))
        }
        _ => Err(invalid(value)),
    }
}

fn invalid(value: &Value) -> ows::Error {
    ows::Error::runtime(
        "Invalid duration",
        format!("{value} isn't a duration of at most 100 years."),
    )
}

/// A number of seconds as a duration, if it's finite and at most [`MAX`].
fn bounded(seconds: f64) -> Option<Duration> {
    Duration::try_from_secs_f64(seconds)
        .ok()
        .filter(|duration| *duration <= MAX)
}

/// A plain decimal number, such as `2` or `0.5`: digits with at most one point.
fn decimal(text: &str) -> Option<f64> {
    let digits = text.chars().filter(char::is_ascii_digit).count();
    let points = text.chars().filter(|c| *c == '.').count();
    (digits > 0 && points <= 1 && digits + points == text.len())
        .then(|| text.parse().ok())
        .flatten()
}

/// `P[nD][T[nH][nM][nS]]`, with an optional fraction on the last number. Weeks,
/// months and years aren't fixed lengths, so they aren't accepted.
fn iso8601(text: &str) -> Option<Duration> {
    let rest = text.strip_prefix('P')?;
    let (date, time) = match rest.split_once('T') {
        Some((date, time)) => (date, Some(time)),
        None => (rest, None),
    };
    let mut seconds = 0.0;
    let mut parts = 0;
    for (part, units) in [
        (date, &[('D', 86_400.0)][..]),
        (
            time.unwrap_or(""),
            &[('H', 3_600.0), ('M', 60.0), ('S', 1.0)][..],
        ),
    ] {
        let mut rest = part;
        for (unit, scale) in units {
            if let Some((number, after)) = rest.split_once(*unit) {
                let number = decimal(number)?;
                seconds += number * scale;
                rest = after;
                parts += 1;
            }
        }
        if !rest.is_empty() {
            return None;
        }
    }
    if parts == 0 || time == Some("") {
        return None;
    }
    bounded(seconds)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_both_forms() {
        assert_eq!(parse(&json!("PT1M30S")), Ok(Duration::from_secs(90)));
        assert_eq!(parse(&json!("P1DT1H")), Ok(Duration::from_secs(90_000)));
        assert_eq!(parse(&json!("PT0.5S")), Ok(Duration::from_millis(500)));
        assert_eq!(
            parse(&json!({"minutes": 1, "milliseconds": 500})),
            Ok(Duration::from_millis(60_500))
        );
        assert_eq!(parse(&json!("P36500D")), Ok(MAX));
        assert_eq!(parse(&json!({"days": 36_500})), Ok(MAX));
    }

    #[test]
    fn rejects_what_isnt_a_duration() {
        for value in [
            json!("1m"),
            json!("P"),
            json!("PT"),
            json!("P1W"),
            json!("P1M"),
            json!("P1Y"),
            json!({"weeks": 1}),
            json!(5),
            json!("PTinfS"),
            json!("PTNaNS"),
            json!("PT1e30S"),
            json!("PT-1S"),
            json!("PT+1S"),
            json!("PT1..5S"),
            json!("P300000000000000D"),
            json!({"days": 300_000_000_000_000_u64}),
            json!({"days": 36_501}),
            json!({"seconds": 1e300, "days": 1e300}),
        ] {
            assert!(parse(&value).is_err(), "{value}");
        }
    }
}
