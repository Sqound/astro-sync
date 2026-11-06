//! What the service says of the billing of a space, as far as Astro Sync
//! needs to say it: **a payment that failed**.
//!
//! The rule: a subscription whose payment failed does not take the space
//! away. From the day the payment provider put the subscription on hold, the
//! space is **read-only** until the date the service gives -- it keeps
//! everything, exports everything, and accepts nothing new -- then it follows
//! the ordinary end of a subscription. Astro Sync sells and counts nothing; it
//! only has to say, in the words of the application, why what it sends is
//! held, and until when.
//!
//! The service says it in `GET /v1/entitlements`: the purchase keeps the state `on_hold`, with
//! `endedAt` (the day the hold started) and `readOnlyUntil` (the end of the read-only window).
//! Astro Sync reads exactly that and nothing else of the answer -- not the
//! purchases, not the offers, not the capabilities of the same body (the map
//! of `GET /v1/capabilities` is the one it keeps). The answers it plays in its
//! tests are its own, reduced to what it reads
//! (`tests/fixtures/standing-answers.json`). An ended subscription
//! (`expired_read_only`) needs no reading: the refusal of an upload
//! (`403` `entitlement_read_only`) already says it.
//!
//! The standing is kept beside the capabilities, in the same cache file, so
//! that a computer offline for a week still says it. It is never expired by
//! the core: the interface compares `readOnlyUntil` with its own clock (a
//! window that is over says nothing).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How the billing of the space limits what Astro Sync may send.
///
/// Serialized as the interface reads it: `{ "kind": "paymentFailed", "since":
/// "...", "readOnlyUntil": "..." }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Standing {
    /// The payment of the subscription failed and the retry goes on: the
    /// space is read-only until `read_only_until`.
    PaymentFailed {
        /// The day the hold started (`endedAt`), as the service wrote it.
        since: String,
        /// The end of the read-only window (`readOnlyUntil`), as the service
        /// wrote it.
        read_only_until: String,
    },
}

/// Whether `text` is written as an instant of the contract
/// (`2026-12-29T12:00:00.000Z`): a date, a `T`, a time. Anything else is not
/// shown to a person as a date.
fn is_instant(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 20
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
        && bytes[10] == b'T'
}

fn instant(purchase: &Value, field: &str) -> Option<String> {
    purchase
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| is_instant(text))
        .map(str::to_owned)
}

/// Reads the body of `GET /v1/entitlements` as the standing it carries.
///
/// - A subscription **in force** (`subscription` is present: `active` or
///   `grace`, possibly paid again after the failure) leaves nothing to say.
/// - A space flagged `serviceAdministrator` either.
/// - Otherwise a subscription purchase in the state `on_hold` with its two
///   dates is a payment that failed. If several are, the window that ends
///   last is the one said (the instants are all written by the same service
///   in the same form, so they compare as text).
/// - Anything else -- a free space, one-time purchases, an ended
///   subscription, a private server's answer -- says nothing: `None`.
///
/// `Err` only for a body that is not the answer of the route (no `purchases`
/// list).
pub fn standing_of_entitlements(body: &Value) -> Result<Option<Standing>, String> {
    let purchases = body
        .get("purchases")
        .and_then(Value::as_array)
        .ok_or_else(|| "the list of purchases is missing from the answer".to_string())?;
    if body.get("subscription").is_some_and(|live| !live.is_null())
        || body.get("serviceAdministrator").and_then(Value::as_bool) == Some(true)
    {
        return Ok(None);
    }
    let standing = purchases
        .iter()
        .filter(|purchase| {
            purchase.get("kind").and_then(Value::as_str) == Some("subscription")
                && purchase.get("state").and_then(Value::as_str) == Some("on_hold")
        })
        .filter_map(|purchase| {
            Some((
                instant(purchase, "endedAt")?,
                instant(purchase, "readOnlyUntil")?,
            ))
        })
        .max_by(|left, right| left.1.cmp(&right.1))
        .map(|(since, read_only_until)| Standing::PaymentFailed {
            since,
            read_only_until,
        });
    Ok(standing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hold(ended: &str, until: &str) -> Value {
        json!({
            "id": "8f2c6d41-7a3e-4b95-9d10-5e6a1c3b7f82",
            "sku": "synchronization",
            "kind": "subscription",
            "state": "on_hold",
            "currentPeriodEnd": ended,
            "channel": "web",
            "endedAt": ended,
            "readOnlyUntil": until,
        })
    }

    fn body(purchases: Vec<Value>) -> Value {
        json!({ "purchases": purchases, "capabilities": {} })
    }

    #[test]
    fn a_subscription_on_hold_is_a_payment_that_failed_until_the_end_of_its_window() {
        let answer = body(vec![hold(
            "2026-09-30T12:00:00.000Z",
            "2026-12-29T12:00:00.000Z",
        )]);
        assert_eq!(
            standing_of_entitlements(&answer).unwrap(),
            Some(Standing::PaymentFailed {
                since: "2026-09-30T12:00:00.000Z".to_string(),
                read_only_until: "2026-12-29T12:00:00.000Z".to_string(),
            })
        );
    }

    #[test]
    fn it_is_serialized_as_the_interface_reads_it() {
        let standing = Standing::PaymentFailed {
            since: "2026-09-30T12:00:00.000Z".to_string(),
            read_only_until: "2026-12-29T12:00:00.000Z".to_string(),
        };
        assert_eq!(
            serde_json::to_value(&standing).unwrap(),
            json!({
                "kind": "paymentFailed",
                "since": "2026-09-30T12:00:00.000Z",
                "readOnlyUntil": "2026-12-29T12:00:00.000Z",
            })
        );
        let back: Standing =
            serde_json::from_value(serde_json::to_value(&standing).unwrap()).unwrap();
        assert_eq!(back, standing);
    }

    #[test]
    fn a_subscription_in_force_or_an_administrator_says_nothing() {
        let mut answer = body(vec![hold(
            "2026-09-30T12:00:00.000Z",
            "2026-12-29T12:00:00.000Z",
        )]);
        answer["subscription"] = json!({
            "sku": "synchronization",
            "period": "monthly",
            "currentPeriodEnd": "2026-11-30T12:00:00.000Z",
        });
        assert_eq!(standing_of_entitlements(&answer).unwrap(), None);

        let mut admin = body(vec![hold(
            "2026-09-30T12:00:00.000Z",
            "2026-12-29T12:00:00.000Z",
        )]);
        admin["serviceAdministrator"] = json!(true);
        assert_eq!(standing_of_entitlements(&admin).unwrap(), None);
    }

    #[test]
    fn what_is_not_a_hold_with_its_dates_says_nothing() {
        // No purchase at all, a one-time purchase, an ended subscription.
        assert_eq!(standing_of_entitlements(&body(vec![])).unwrap(), None);
        let mut ended = hold("2026-09-30T12:00:00.000Z", "2026-12-29T12:00:00.000Z");
        ended["state"] = json!("expired_read_only");
        assert_eq!(standing_of_entitlements(&body(vec![ended])).unwrap(), None);
        let mut one_time = hold("2026-09-30T12:00:00.000Z", "2026-12-29T12:00:00.000Z");
        one_time["kind"] = json!("one_time");
        assert_eq!(
            standing_of_entitlements(&body(vec![one_time])).unwrap(),
            None
        );
        // A hold that does not say its dates, or says them as something that
        // is not an instant, is not shown as a date.
        let mut undated = hold("2026-09-30T12:00:00.000Z", "2026-12-29T12:00:00.000Z");
        undated.as_object_mut().unwrap().remove("readOnlyUntil");
        assert_eq!(
            standing_of_entitlements(&body(vec![undated])).unwrap(),
            None
        );
        let odd = hold("2026-09-30T12:00:00.000Z", "soon");
        assert_eq!(standing_of_entitlements(&body(vec![odd])).unwrap(), None);
    }

    #[test]
    fn of_two_holds_the_window_that_ends_last_is_said() {
        let answer = body(vec![
            hold("2026-08-01T00:00:00.000Z", "2026-10-30T00:00:00.000Z"),
            hold("2026-09-30T12:00:00.000Z", "2026-12-29T12:00:00.000Z"),
        ]);
        assert_eq!(
            standing_of_entitlements(&answer).unwrap(),
            Some(Standing::PaymentFailed {
                since: "2026-09-30T12:00:00.000Z".to_string(),
                read_only_until: "2026-12-29T12:00:00.000Z".to_string(),
            })
        );
    }

    #[test]
    fn a_body_without_the_list_of_purchases_is_not_the_answer_of_the_route() {
        assert!(standing_of_entitlements(&json!({ "capabilities": {} })).is_err());
        assert!(standing_of_entitlements(&json!("nope")).is_err());
    }
}
