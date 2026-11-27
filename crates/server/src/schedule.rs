//! Cron patterns in a time zone: when a backup runs next.

use std::str::FromStr;

use axum::{Json, extract::Query};
use chrono_tz::Tz;
use croner::Cron;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{Error, auth::Auth};

/// The first run after `after`. Five fields only, so nothing runs more than once a minute.
pub fn next_after(
    pattern: &str,
    timezone: &str,
    after: OffsetDateTime,
) -> Result<OffsetDateTime, Error> {
    if pattern.len() > 100 || pattern.split_whitespace().count() != 5 {
        return Err(Error::Invalid("schedule"));
    }
    let cron = Cron::from_str(pattern).map_err(|_| Error::Invalid("schedule"))?;
    let zone = Tz::from_str(timezone).map_err(|_| Error::Invalid("timezone"))?;
    let after = chrono::DateTime::from_timestamp(after.unix_timestamp(), 0)
        .ok_or(Error::Invalid("schedule"))?
        .with_timezone(&zone);
    // A pattern such as 30 February never matches.
    let next = cron
        .find_next_occurrence(&after, false)
        .map_err(|_| Error::Invalid("schedule"))?;
    OffsetDateTime::from_unix_timestamp(next.timestamp()).map_err(|_| Error::Invalid("schedule"))
}

#[derive(Deserialize)]
pub struct Preview {
    schedule: String,
    timezone: String,
}

#[derive(Serialize)]
pub struct Runs {
    #[serde(with = "rfc3339_list")]
    runs: Vec<OffsetDateTime>,
}

mod rfc3339_list {
    use serde::{Serializer, ser::Error};
    use time::{OffsetDateTime, format_description::well_known::Rfc3339};

    pub fn serialize<S: Serializer>(list: &[OffsetDateTime], to: S) -> Result<S::Ok, S::Error> {
        let text = list
            .iter()
            .map(|t| t.format(&Rfc3339))
            .collect::<Result<Vec<_>, _>>()
            .map_err(S::Error::custom)?;
        to.collect_seq(text)
    }
}

pub async fn preview(Auth(_): Auth, Query(query): Query<Preview>) -> Result<Json<Runs>, Error> {
    let mut at = OffsetDateTime::now_utc();
    let mut runs = Vec::with_capacity(3);
    for _ in 0..3 {
        at = next_after(&query.schedule, &query.timezone, at)?;
        runs.push(at);
    }
    Ok(Json(Runs { runs }))
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::next_after;

    #[test]
    fn runs_follow_the_time_zone() {
        let from = datetime!(2026-01-31 12:00 UTC);
        let next = |pattern, zone| next_after(pattern, zone, from).ok();
        // 01:00 in Jakarta is 18:00 UTC the day before.
        assert_eq!(
            next("0 1 * * *", "Asia/Jakarta"),
            Some(datetime!(2026-01-31 18:00 UTC))
        );
        assert_eq!(
            next("0 */6 * * *", "UTC"),
            Some(datetime!(2026-01-31 18:00 UTC))
        );
        // Months without the day are skipped.
        assert_eq!(
            next("0 0 31 * *", "UTC"),
            Some(datetime!(2026-03-31 00:00 UTC))
        );
        for (pattern, zone) in [
            ("0 1 * *", "UTC"),
            ("* * * * * *", "UTC"),
            ("61 1 * * *", "UTC"),
            ("0 0 30 2 *", "UTC"),
            ("0 1 * * *", "Mars/Base"),
        ] {
            assert!(next(pattern, zone).is_none(), "{pattern} {zone}");
        }
    }
}
