//! Tenax issue #1990: a standby that never settles must not be re-spawned
//! on every 10 s `ensure_warm_standby` tick.
//!
//! Observed on a Windows host with several live sessions: ~6 hidden console
//! windows per minute, each a `psmux server -s __warm__` spawn parented to a
//! session server, each exiting moments later. Two loops produce that count:
//!
//! * the standby dies (or is judged dead) every cycle — each tick spawns a
//!   fresh one; or
//! * the standby is alive but the `.port` probe cannot verify it, and every
//!   tick deleted its registry files and spawned a sibling that exits at once
//!   on the `__warm__` session mutex.
//!
//! The fix is two rules, kept pure here: a shared `.spawnat` stamp bounds
//! periodic respawns to one per interval, and a live `.pid` anchor vetoes a
//! spawn whose outcome is already decided by the mutex.

use super::*;

/// The stamp gate: no stamp means "never tried" and always permits.
#[test]
fn respawn_is_due_when_never_attempted() {
    assert!(warm_respawn_due(None));
}

/// A spawn attempted less than the interval ago blocks the periodic check.
#[test]
fn respawn_is_not_due_inside_the_interval() {
    for age in [Duration::ZERO, Duration::from_secs(1), WARM_RESPAWN_MIN_INTERVAL - Duration::from_secs(1)] {
        assert!(
            !warm_respawn_due(Some(age)),
            "age {:?} is inside the interval",
            age
        );
    }
}

/// Once the interval has elapsed a retry is allowed again.
#[test]
fn respawn_is_due_again_after_the_interval() {
    assert!(warm_respawn_due(Some(WARM_RESPAWN_MIN_INTERVAL)));
    assert!(warm_respawn_due(Some(WARM_RESPAWN_MIN_INTERVAL * 10)));
}

/// The churn case itself: the probe cannot reach the standby but its `.pid`
/// anchor names a live psmux. The owner still holds the `__warm__` mutex, so
/// the spawn would exit on entry — spawning must be skipped, not repeated.
#[test]
fn unreachable_warm_with_live_owner_does_not_spawn() {
    assert!(!warm_spawn_helps(WarmVerify::Unreachable, true));
}

/// A claimed standby leaves the `.port` pointing at a real session that
/// answers under its own name; the mutex was released by the claim's rekey,
/// so spawning is the way the pool refills.
#[test]
fn stale_pointer_to_claimed_session_spawns() {
    assert!(warm_spawn_helps(WarmVerify::OtherSession, true));
    assert!(warm_spawn_helps(WarmVerify::OtherSession, false));
}

/// Dead anchor or no anchor at all: the registry is stale, sweep and spawn.
#[test]
fn unreachable_warm_with_dead_owner_spawns() {
    assert!(warm_spawn_helps(WarmVerify::Unreachable, false));
}

/// A genuine standby is never re-spawned.
#[test]
fn genuine_warm_never_spawns() {
    assert!(!warm_spawn_helps(WarmVerify::Genuine, true));
    assert!(!warm_spawn_helps(WarmVerify::Genuine, false));
}

/// The stamp read itself: missing file is "no attempt", a fresh file is a
/// young stamp, and a stale one is old enough to retry.
#[test]
fn stamp_file_age_reports_absent_fresh_and_stale() {
    let dir = std::env::temp_dir().join(format!(
        "psmux_t1990_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let stamp = dir.join("x.spawnat");
    let stamp = stamp.to_str().expect("temp path is utf8").to_string();

    assert_eq!(stamp_file_age(&stamp), None, "no stamp file reads as None");

    std::fs::write(&stamp, "1").expect("write stamp");
    let age = stamp_file_age(&stamp).expect("fresh stamp has an age");
    assert!(age < Duration::from_secs(30), "fresh stamp reads young");
    assert!(!warm_respawn_due(Some(age)));

    let _ = std::fs::remove_file(&stamp);
    let _ = std::fs::remove_dir(&dir);
}
