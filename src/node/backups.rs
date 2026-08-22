//! When this node backs itself up, where the copy goes, and how long it
//! is kept.
//!
//! `backup` was a command and only a command. Everything it needed was
//! an argument or a constant: the destination came from `--out`, the
//! recovery window from `KEEP_DAYS`, and *when* from whoever remembered
//! to type it. That is the shape of a feature nobody sets up — and a
//! backup nobody scheduled is the backup that does not exist on the day
//! the disk does not come back.
//!
//! So the decision is a row on the node it binds, like every other
//! decision here, and two doors are thin over it: the form on the
//! console's Backup tab, and the loop in [`crate::commands::serve`] that
//! asks whether one is owed. Taking one is
//! [`crate::commands::backup::once`], which is the same code the command
//! runs — same rule as `network::join`.
//!
//! ## What is deliberately not here
//!
//! The S3 credential. It stays in `config.toml` and the reasoning is in
//! [`crate::config::BackupConfig`]: a secret that can read every backup
//! in the network must not be inside the thing being backed up.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use wabot::sqlite::{SqliteDatabase, SqliteResult};

use super::settings::{read, write};
use crate::config::Config;

const DESTINATION: &str = "backup.destination";
const CADENCE: &str = "backup.cadence";
const HOUR: &str = "backup.hour";
const WEEKDAY: &str = "backup.weekday";
const KEEP_DAYS: &str = "backup.keep_days";
const LAST_ATTEMPT: &str = "backup.last_attempt";
const LAST_OK: &str = "backup.last_ok";
const LAST_NOTE: &str = "backup.last_note";
const LAST_ERROR: &str = "backup.last_error";

/// A day in milliseconds. Exact, because the node keeps UTC and UTC has
/// no daylight saving — which is what lets the next slot be arithmetic
/// rather than a walk over a calendar.
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// How often a backup is taken, if at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Cadence {
    #[default]
    Off,
    Daily,
    Weekly,
}

impl Cadence {
    /// The stored spelling, and the form's value with it.
    ///
    /// **Empty for `Off`**, which is not a shortcut. The `fields` island
    /// reads `data-when="cadence"` on a select as "it has a value", so an
    /// empty `Off` is what hides the hour when nothing is scheduled — one
    /// attribute rather than a second condition and another script.
    pub fn as_str(self) -> &'static str {
        match self {
            Cadence::Off => "",
            Cadence::Daily => "daily",
            Cadence::Weekly => "weekly",
        }
    }

    /// Anything this binary does not know is off.
    ///
    /// A value written by a newer version, or by a hand in `sqlite3`,
    /// must not become a schedule nobody chose — and off is the answer
    /// that spends no disk and starts no `pg_basebackup`.
    pub fn parse(text: &str) -> Self {
        match text.trim() {
            "daily" => Cadence::Daily,
            "weekly" => Cadence::Weekly,
            _ => Cadence::Off,
        }
    }
}

/// What this node has decided about backing itself up.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Plan {
    /// Where the copy goes, as an operator wrote it: `ssh://…`, `s3://…`
    /// or a path. `None` means the local default under
    /// `data_dir/backups`, which is where `backup` with no `--out` has
    /// always written — and which protects against nothing that has ever
    /// happened to a disk, so the form says so.
    pub destination: Option<String>,
    pub cadence: Cadence,
    /// The hour, **UTC**. The node keeps UTC and the console says so
    /// beside the field: an operator in another zone who picks 03:00 and
    /// gets their own afternoon has scheduled a `pg_basebackup` of every
    /// database through their busiest hour.
    pub hour: u8,
    /// Days from Monday, read only by a weekly plan.
    pub weekday: u8,
    /// How far back a restore can reach. The window `sweep` bounds the
    /// archive by — see [`crate::commands::backup::keeping`], which is
    /// where the anchor rule lives.
    pub keep_days: i64,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            destination: None,
            // **Off, and an upgrade does not turn it on.** Unlike the
            // write-ahead log — whose default flipped once there was
            // pruning — this writes a copy of every volume somewhere, and
            // the somewhere is a decision: a schedule that appeared with
            // a release would fill the disk it was meant to protect, on a
            // node whose operator asked for neither.
            cadence: Cadence::Off,
            // Not midnight. That is where every cron job anybody has ever
            // written already is, and a one-core node meeting all of them
            // at once is the hour to avoid rather than the one to pick.
            hour: 3,
            weekday: SUNDAY,
            keep_days: crate::commands::backup::KEEP_DAYS,
        }
    }
}

/// Days from Monday, which is what `time` counts in.
pub const SUNDAY: u8 = 6;

/// The floor on the recovery window.
///
/// One day, because zero is not a shorter window — it is `sweep`
/// deleting the backup it just took, and an archive with nothing to
/// replay onto.
pub const MIN_KEEP_DAYS: i64 = 1;

/// What this node has decided, or the defaults where it has not said.
pub async fn plan(database: &SqliteDatabase) -> Plan {
    let default = Plan::default();
    Plan {
        destination: text(database, DESTINATION).await,
        cadence: Cadence::parse(&text(database, CADENCE).await.unwrap_or_default()),
        hour: number(database, HOUR).await.unwrap_or(default.hour).min(23),
        weekday: number(database, WEEKDAY)
            .await
            .unwrap_or(default.weekday)
            .min(SUNDAY),
        keep_days: number(database, KEEP_DAYS)
            .await
            .unwrap_or(default.keep_days)
            .max(MIN_KEEP_DAYS),
    }
}

/// Store it.
///
/// Written whole rather than field by field: the form submits every
/// answer at once, and a partial write would leave a schedule whose hour
/// belongs to the plan before it.
pub async fn store(database: &SqliteDatabase, plan: &Plan) -> SqliteResult<()> {
    write(
        database,
        DESTINATION,
        plan.destination.as_deref().unwrap_or_default(),
    )
    .await?;
    write(database, CADENCE, plan.cadence.as_str()).await?;
    write(database, HOUR, &plan.hour.min(23).to_string()).await?;
    write(database, WEEKDAY, &plan.weekday.min(SUNDAY).to_string()).await?;
    write(
        database,
        KEEP_DAYS,
        &plan.keep_days.max(MIN_KEEP_DAYS).to_string(),
    )
    .await
}

/// How the last one went.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Attempt {
    /// When one was last tried, whatever came of it.
    pub at: Option<i64>,
    /// When one last worked. Kept apart from `at` so that a run failing
    /// today cannot hide the fact that the newest copy is nine days old
    /// — which is the question somebody is really asking.
    pub ok_at: Option<i64>,
    /// What the last good one held, in the words it reported.
    pub note: Option<String>,
    /// Why the last one failed, from whatever refused it. Never
    /// translated: it is rsync's words, or S3's, or a filesystem's.
    pub error: Option<String>,
}

pub async fn last(database: &SqliteDatabase) -> Attempt {
    Attempt {
        at: number(database, LAST_ATTEMPT).await,
        ok_at: number(database, LAST_OK).await,
        note: text(database, LAST_NOTE).await,
        error: text(database, LAST_ERROR).await,
    }
}

/// Record how one went.
///
/// **The failure is stored, not only logged.** A backup that has been
/// refused for a week is invisible in a journal nobody is reading, and
/// this is the row the page and the attention card read.
pub async fn record(
    database: &SqliteDatabase,
    at: i64,
    outcome: &Result<String, String>,
) -> SqliteResult<()> {
    write(database, LAST_ATTEMPT, &at.to_string()).await?;
    match outcome {
        Ok(note) => {
            write(database, LAST_OK, &at.to_string()).await?;
            write(database, LAST_NOTE, note).await?;
            // Cleared: a reason left behind after a run that worked is a
            // page reporting a failure that has been fixed.
            write(database, LAST_ERROR, "").await
        }
        Err(reason) => write(database, LAST_ERROR, &one_line(reason)).await,
    }
}

/// The first line of it, and not more than a paragraph.
///
/// What refused a backup can be a page of output — `pg_basebackup` says
/// the same sentence twice, once per encryption attempt — and this row is
/// read by two things that put it in a sentence: the Backup tab and
/// `doctor`. The journal keeps the whole of it; a row that holds a page
/// makes both of those unreadable, which is how a reason nobody can scan
/// becomes a reason nobody reads.
fn one_line(reason: &str) -> String {
    const MOST: usize = 300;
    let line = reason
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_string();
    match line.char_indices().nth(MOST) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line,
    }
}

/// The most recent moment the plan asked for, at or before `now_ms`.
///
/// **A slot, not an interval.** "Twenty-four hours since the last one"
/// drifts — a node down for an hour moves its backup an hour later for
/// ever — and it cannot answer the one question an operator wants to
/// decide, which is what time of day this happens. A slot also catches
/// up: a machine that was off at 03:00 takes the backup when it comes
/// back, because that slot is still the most recent one and nothing has
/// claimed it.
pub fn slot_at_or_before(plan: &Plan, now_ms: i64) -> Option<i64> {
    if plan.cadence == Cadence::Off {
        return None;
    }
    let now = time::OffsetDateTime::from_unix_timestamp(now_ms / 1000).ok()?;
    let mut candidate = at_hour(now.date(), plan.hour)?;
    if candidate > now {
        candidate = candidate.checked_sub(time::Duration::days(1))?;
    }
    if plan.cadence == Cadence::Weekly {
        // Back to the wanted day. Never forward: a slot in the future is
        // not a slot, and rounding up would take a backup a week early
        // the first time a plan is saved.
        let back = i64::from(
            (candidate.weekday().number_days_from_monday() + 7 - plan.weekday.min(SUNDAY)) % 7,
        );
        candidate = candidate.checked_sub(time::Duration::days(back))?;
    }
    Some(candidate.unix_timestamp() * 1000)
}

/// The next one, for a page that says when it will happen.
pub fn next_slot(plan: &Plan, now_ms: i64) -> Option<i64> {
    let previous = slot_at_or_before(plan, now_ms)?;
    let step = match plan.cadence {
        Cadence::Weekly => 7,
        _ => 1,
    };
    Some(previous + step * DAY_MS)
}

/// Whether a backup is owed.
///
/// `last` is the last **attempt**, not the last success. A destination
/// that refuses, retried on every pass, is a `pg_basebackup` of every
/// database every five minutes on a machine with one core — so a slot
/// gets one attempt, the reason lands on the row and on the page, and
/// the button beside it is how somebody retries on purpose.
pub fn due(plan: &Plan, last: Option<i64>, now_ms: i64) -> bool {
    let Some(slot) = slot_at_or_before(plan, now_ms) else {
        return false;
    };
    last.is_none_or(|at| at < slot)
}

/// Take one now, and record how it went.
///
/// The one implementation both doors are thin over: the button on the
/// node's Backup tab and the scheduled loop. Never two at once — see
/// [`in_progress`].
pub async fn take_now(config: &Config, database: &SqliteDatabase) -> Result<String, String> {
    let Some(_running) = Running::claim() else {
        return Err("a backup is already running on this node".into());
    };
    let plan = plan(database).await;
    let at = crate::platform::now_ms();
    let outcome = crate::commands::backup::once(config, database, plan.destination.clone()).await;
    if let Err(error) = record(database, at, &outcome).await {
        tracing::warn!(%error, "could not record how the backup went");
    }
    outcome
}

/// Start one and answer the request now.
///
/// A backup is minutes of `pg_basebackup` and file copying, and a browser
/// waiting on that times out somewhere in the middle — leaving the
/// operator with no page and a backup still running. Same shape as
/// `update::start_in_background`, and for the same reason.
///
/// Detached, so a shutdown in the middle leaves a part-written directory
/// behind. That is the harmless direction: each run writes its own
/// timestamped one, `restore` refuses a directory with no
/// `manifest.json` in it, and `clean` names what nothing claims.
pub fn start_in_background(config: Config, database: Arc<SqliteDatabase>) {
    tokio::spawn(async move {
        match take_now(&config, &database).await {
            Ok(held) => tracing::info!(%held, "backup taken"),
            Err(reason) => tracing::warn!(%reason, "the backup did not work"),
        }
    });
}

/// Whether one is running right now.
///
/// A process-wide flag rather than a row: both callers are in this
/// process, and what must not happen twice is the *work* — two
/// `pg_basebackup`s of the same database on a one-core node, into two
/// staging directories, is a machine that stops answering. A row would
/// also have to be tidied up after a crash, where a flag that dies with
/// the process cannot lie about a run that is not running.
pub fn in_progress() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

static RUNNING: AtomicBool = AtomicBool::new(false);

/// The flag, released however the run ends.
struct Running;

impl Running {
    fn claim() -> Option<Self> {
        RUNNING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| Running)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

/// A stored string, or `None` where nothing has been said. Empty is
/// nothing, which is what clearing a field means.
async fn text(database: &SqliteDatabase, key: &str) -> Option<String> {
    match read(database, key).await {
        Ok(stored) => stored
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        Err(error) => {
            tracing::warn!(%error, key, "could not read a backup setting");
            None
        }
    }
}

async fn number<T: std::str::FromStr>(database: &SqliteDatabase, key: &str) -> Option<T> {
    text(database, key)
        .await
        .and_then(|value| value.parse().ok())
}

fn at_hour(day: time::Date, hour: u8) -> Option<time::OffsetDateTime> {
    let time = time::Time::from_hms(hour.min(23), 0, 0).ok()?;
    Some(time::PrimitiveDateTime::new(day, time).assume_utc())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-08-21 is a Friday. Every moment below is UTC.
    const FRIDAY_NOON: i64 = 1_787_313_600_000;

    fn at(text: &str) -> i64 {
        // `2026-08-21 12:00` as milliseconds, worked out by hand for the
        // few moments these tests need. A parser here would be a second
        // implementation of the thing under test.
        match text {
            "friday 12:00" => FRIDAY_NOON,
            "friday 02:00" => FRIDAY_NOON - 10 * 3_600_000,
            "friday 03:00" => FRIDAY_NOON - 9 * 3_600_000,
            other => panic!("no moment named {other}"),
        }
    }

    fn daily(hour: u8) -> Plan {
        Plan {
            cadence: Cadence::Daily,
            hour,
            ..Plan::default()
        }
    }

    #[test]
    fn nothing_is_owed_while_nothing_is_scheduled() {
        let plan = Plan::default();
        assert_eq!(plan.cadence, Cadence::Off);
        assert_eq!(slot_at_or_before(&plan, at("friday 12:00")), None);
        assert!(!due(&plan, None, at("friday 12:00")));
    }

    /// The slot is today's hour once it has passed, and yesterday's
    /// before it — a plan saved at breakfast must not claim the backup
    /// it has not taken yet.
    #[test]
    fn the_slot_is_the_last_one_that_has_come_round() {
        let plan = daily(3);
        assert_eq!(
            slot_at_or_before(&plan, at("friday 12:00")),
            Some(at("friday 03:00"))
        );
        assert_eq!(
            slot_at_or_before(&plan, at("friday 02:00")),
            Some(at("friday 03:00") - DAY_MS)
        );
        // The hour itself counts as arrived.
        assert_eq!(
            slot_at_or_before(&plan, at("friday 03:00")),
            Some(at("friday 03:00"))
        );
    }

    /// A machine that was off at three o'clock takes the backup when it
    /// comes back, rather than skipping the day.
    #[test]
    fn a_slot_nothing_claimed_is_still_owed() {
        let plan = daily(3);
        assert!(due(&plan, None, at("friday 12:00")));
        assert!(due(
            &plan,
            Some(at("friday 03:00") - DAY_MS),
            at("friday 12:00")
        ));
        // And once it has been taken, the day is done.
        assert!(!due(&plan, Some(at("friday 03:00")), at("friday 12:00")));
    }

    /// A destination that refuses gets one attempt per slot.
    ///
    /// The failing run still counts, which is the whole point: retrying
    /// every pass is a `pg_basebackup` of every database every five
    /// minutes, and the reason is on the page for somebody to act on.
    #[test]
    fn a_failed_attempt_waits_for_the_next_slot() {
        let plan = daily(3);
        let attempted = at("friday 03:00") + 60_000;
        assert!(!due(&plan, Some(attempted), at("friday 12:00")));
        assert!(due(&plan, Some(attempted), at("friday 12:00") + DAY_MS));
    }

    /// Weekly walks back to the day it was told, never forward.
    #[test]
    fn a_weekly_slot_is_the_last_time_that_day_came_round() {
        let sunday = Plan {
            cadence: Cadence::Weekly,
            hour: 3,
            weekday: SUNDAY,
            ..Plan::default()
        };
        // Friday noon: the last Sunday at 03:00 was five days ago.
        assert_eq!(
            slot_at_or_before(&sunday, at("friday 12:00")),
            Some(at("friday 03:00") - 5 * DAY_MS)
        );
        // And the next one is two days out, not seven.
        assert_eq!(
            next_slot(&sunday, at("friday 12:00")),
            Some(at("friday 03:00") + 2 * DAY_MS)
        );

        // Asked on the day itself, before the hour: last week's.
        let friday = Plan {
            weekday: 4,
            ..sunday.clone()
        };
        assert_eq!(
            slot_at_or_before(&friday, at("friday 02:00")),
            Some(at("friday 03:00") - 7 * DAY_MS)
        );
        assert_eq!(
            slot_at_or_before(&friday, at("friday 12:00")),
            Some(at("friday 03:00"))
        );
    }

    #[tokio::test]
    async fn a_plan_survives_the_round_trip() {
        let database = crate::db::open_in_memory().await.expect("open");
        assert_eq!(plan(&database).await, Plan::default());

        let wanted = Plan {
            destination: Some("ssh://backups.example/srv/wabot".into()),
            cadence: Cadence::Weekly,
            hour: 4,
            weekday: 2,
            keep_days: 30,
        };
        store(&database, &wanted).await.expect("store");
        assert_eq!(plan(&database).await, wanted);
    }

    /// Empty is "keep it here", not a path named "".
    #[tokio::test]
    async fn a_cleared_destination_is_the_default_one() {
        let database = crate::db::open_in_memory().await.expect("open");
        store(
            &database,
            &Plan {
                destination: Some("  ".into()),
                ..Plan::default()
            },
        )
        .await
        .expect("store");
        assert_eq!(plan(&database).await.destination, None);
    }

    /// A window of nought days is `sweep` deleting the backup it just
    /// took, so the floor holds whatever is stored.
    #[tokio::test]
    async fn the_window_cannot_be_nothing() {
        let database = crate::db::open_in_memory().await.expect("open");
        store(
            &database,
            &Plan {
                keep_days: 0,
                ..Plan::default()
            },
        )
        .await
        .expect("store");
        assert_eq!(plan(&database).await.keep_days, MIN_KEEP_DAYS);
    }

    /// A run that works clears the reason the last one failed.
    #[tokio::test]
    async fn a_good_run_clears_the_last_failure() {
        let database = crate::db::open_in_memory().await.expect("open");
        record(&database, 1_000, &Err("no route to host".into()))
            .await
            .expect("record");
        let failed = last(&database).await;
        assert_eq!(failed.at, Some(1_000));
        assert_eq!(failed.ok_at, None);
        assert_eq!(failed.error.as_deref(), Some("no route to host"));

        record(&database, 2_000, &Ok("12 MB in 2 volume(s)".into()))
            .await
            .expect("record");
        let worked = last(&database).await;
        assert_eq!(worked.ok_at, Some(2_000));
        assert_eq!(worked.error, None);
        assert_eq!(worked.note.as_deref(), Some("12 MB in 2 volume(s)"));
    }

    /// A reason that is a page of output is a row nobody can read.
    ///
    /// `pg_basebackup` prints its refusal once per encryption attempt, so
    /// what came back from the node was the same sentence twice with a
    /// newline between them — and both the tab and `doctor` put this in a
    /// sentence.
    #[tokio::test]
    async fn a_recorded_reason_is_one_line() {
        let database = crate::db::open_in_memory().await.expect("open");
        record(
            &database,
            1_000,
            &Err("pg_basebackup exited 1: no pg_hba.conf entry\nand again, unencrypted".into()),
        )
        .await
        .expect("record");

        let stored = last(&database).await.error.expect("a reason");
        assert_eq!(stored, "pg_basebackup exited 1: no pg_hba.conf entry");
    }

    /// Two at once is two `pg_basebackup`s of the same database.
    #[test]
    fn only_one_runs_at_a_time() {
        assert!(!in_progress());
        let held = Running::claim().expect("nothing else is running");
        assert!(in_progress());
        assert!(Running::claim().is_none(), "a second claim must be refused");
        drop(held);
        assert!(!in_progress());
    }
}
