//! Scheduled tasks: "every weekday at 9 open my calendar and summarise
//! today", "remind me at 5 to send the report".
//!
//! Schedules live in `schedules.json` in the settings folder and are run by
//! the tray process itself (no Windows service, no Task Scheduler entry), so
//! they only fire while Agent Desk is running.
//!
//! Times are wall-clock local times. Everything here works on a plain count
//! of local minutes, so the logic can be tested without a clock or a time
//! zone database; `platform::local_now` supplies the current local time.

use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

/// A repeating schedule that was missed (computer asleep, app closed) still
/// runs if Agent Desk notices within this many minutes. Later than that it
/// waits for its next time, so nobody gets a stale "good morning" at 3 pm.
pub const GRACE_MINUTES: i64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Schedule {
    pub id: u64,
    /// Short label shown in the list. Empty means "use the prompt".
    pub name: String,
    /// What is asked of the AI when the time comes.
    pub prompt: String,
    pub when: When,
    pub paused: bool,
    /// Local minute stamp of the last run (see `stamp`).
    pub last_run: Option<i64>,
    /// Local minute stamp of when it was made. A schedule never catches up
    /// on a time from before it existed.
    pub created: i64,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            prompt: String::new(),
            when: When::Repeat {
                time: "09:00".into(),
                days: vec![],
            },
            paused: false,
            last_run: None,
            created: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum When {
    /// One time only, on `date` ("YYYY-MM-DD") at `time` ("HH:MM").
    Once { date: String, time: String },
    /// At `time` on the given weekdays (0 = Monday … 6 = Sunday). An empty
    /// list means every day.
    Repeat { time: String, days: Vec<u8> },
}

/* ---------- calendar maths ---------- */

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date for a day count from `days_from_civil`.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// A local wall-clock minute as one number, for comparing and adding.
pub fn stamp(y: i64, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
    days_from_civil(y, mo, d) * 1440 + i64::from(h) * 60 + i64::from(mi)
}

/// Monday = 0 … Sunday = 6.
pub fn weekday(stamp: i64) -> u8 {
    // 1970-01-01 was a Thursday.
    (stamp.div_euclid(1440) + 3).rem_euclid(7) as u8
}

pub fn parse_time(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m) = (h.trim().parse::<u32>().ok()?, m.trim().parse::<u32>().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

pub fn parse_date(s: &str) -> Option<(i64, u32, u32)> {
    let mut parts = s.trim().splitn(3, '-');
    let y = parts.next()?.parse::<i64>().ok()?;
    let m = parts.next()?.parse::<u32>().ok()?;
    let d = parts.next()?.parse::<u32>().ok()?;
    if !(1970..=9999).contains(&y) || !(1..=12).contains(&m) || d == 0 {
        return None;
    }
    // Reject 31 February and friends by round-tripping.
    (civil_from_days(days_from_civil(y, m, d)) == (y, m, d)).then_some((y, m, d))
}

const DAY_NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// "Mon 24 Nov, 09:00"
pub fn format_stamp(stamp: i64) -> String {
    let (_, m, d) = civil_from_days(stamp.div_euclid(1440));
    let minute = stamp.rem_euclid(1440);
    format!(
        "{} {} {}, {:02}:{:02}",
        DAY_NAMES[weekday(stamp) as usize],
        d,
        MONTHS[(m - 1) as usize],
        minute / 60,
        minute % 60
    )
}

/// Read a day list such as ["weekdays"], ["mon", "wed"] or ["daily"].
pub fn parse_days(items: &[String]) -> Result<Vec<u8>, String> {
    let mut days = Vec::new();
    for item in items {
        let word = item.trim().to_lowercase();
        let add: &[u8] = match word.as_str() {
            "" => &[],
            "daily" | "every day" | "everyday" | "all" => &[0, 1, 2, 3, 4, 5, 6],
            "weekdays" | "weekday" | "workdays" => &[0, 1, 2, 3, 4],
            "weekends" | "weekend" => &[5, 6],
            _ => {
                let i = DAY_NAMES
                    .iter()
                    .position(|n| word.starts_with(&n.to_lowercase()))
                    .ok_or_else(|| format!("\"{item}\" is not a day of the week."))?;
                days.push(i as u8);
                continue;
            }
        };
        days.extend_from_slice(add);
    }
    days.sort_unstable();
    days.dedup();
    // Every day is stored as "no restriction".
    if days.len() == 7 {
        days.clear();
    }
    Ok(days)
}

impl When {
    pub fn validate(&self) -> Result<(), String> {
        match self {
            When::Once { date, time } => {
                parse_date(date).ok_or("Use a date like 2026-11-24.")?;
                parse_time(time).ok_or("Use a 24-hour time like 17:00.")?;
            }
            When::Repeat { time, days } => {
                parse_time(time).ok_or("Use a 24-hour time like 09:00.")?;
                if days.iter().any(|d| *d > 6) {
                    return Err("Days run from 0 (Monday) to 6 (Sunday).".into());
                }
            }
        }
        Ok(())
    }

    /// "every weekday at 09:00", "once on Mon 24 Nov, 17:00"
    pub fn describe(&self) -> String {
        match self {
            When::Once { date, time } => match (parse_date(date), parse_time(time)) {
                (Some((y, m, d)), Some((h, mi))) => {
                    format!("once, {}", format_stamp(stamp(y, m, d, h, mi)))
                }
                _ => "once (invalid time)".into(),
            },
            When::Repeat { time, days } => {
                let which = match days.as_slice() {
                    [] => "every day".to_string(),
                    [0, 1, 2, 3, 4] => "every weekday".to_string(),
                    [5, 6] => "every weekend day".to_string(),
                    list => format!(
                        "every {}",
                        list.iter()
                            .filter_map(|d| DAY_NAMES.get(*d as usize))
                            .copied()
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                };
                format!("{which} at {}", time.trim())
            }
        }
    }
}

impl Schedule {
    pub fn label(&self) -> &str {
        if self.name.trim().is_empty() {
            self.prompt.trim()
        } else {
            self.name.trim()
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.prompt.trim().is_empty() {
            return Err("Say what Agent Desk should do at that time.".into());
        }
        if self.prompt.chars().count() > 1000 {
            return Err("Keep the task under 1000 characters.".into());
        }
        self.when.validate()
    }

    /// The minute this schedule is meant to run at, for a one-off.
    fn once_slot(&self) -> Option<i64> {
        match &self.when {
            When::Once { date, time } => {
                let (y, m, d) = parse_date(date)?;
                let (h, mi) = parse_time(time)?;
                Some(stamp(y, m, d, h, mi))
            }
            When::Repeat { .. } => None,
        }
    }

    /// Whether a one-off has already run.
    pub fn finished(&self) -> bool {
        matches!(self.when, When::Once { .. }) && self.last_run.is_some()
    }

    /// The repeating time on the day that holds `day_stamp`, if it runs then.
    fn slot_on_day(&self, day: i64) -> Option<i64> {
        let When::Repeat { time, days } = &self.when else {
            return None;
        };
        let (h, mi) = parse_time(time)?;
        let slot = day * 1440 + i64::from(h) * 60 + i64::from(mi);
        (days.is_empty() || days.contains(&weekday(slot))).then_some(slot)
    }

    /// If the schedule should run now, the minute it was due.
    pub fn due(&self, now: i64) -> Option<i64> {
        if self.paused {
            return None;
        }
        let since = self.last_run.unwrap_or(self.created);
        match &self.when {
            // A missed reminder is better late than never: a one-off runs
            // the first time the app sees it is due.
            When::Once { .. } => {
                let slot = self.once_slot()?;
                (self.last_run.is_none() && now >= slot).then_some(slot)
            }
            When::Repeat { .. } => {
                let today = now.div_euclid(1440);
                // Yesterday too, for a 23:50 schedule noticed at 00:05.
                [today, today - 1]
                    .into_iter()
                    .filter_map(|day| self.slot_on_day(day))
                    .find(|&slot| now >= slot && now - slot <= GRACE_MINUTES && slot > since)
            }
        }
    }

    /// The next minute it will run, for showing in the list.
    pub fn next_run(&self, now: i64) -> Option<i64> {
        if self.paused || self.finished() {
            return None;
        }
        match &self.when {
            When::Once { .. } => self.once_slot(),
            When::Repeat { .. } => {
                let since = self.last_run.unwrap_or(self.created).max(now - 1);
                let today = now.div_euclid(1440);
                (today..today + 8)
                    .filter_map(|day| self.slot_on_day(day))
                    .find(|&slot| slot > since)
            }
        }
    }
}

/* ---------- storage ---------- */

const FILE: &str = "schedules.json";

pub fn load(dir: &Path) -> Vec<Schedule> {
    fs::read_to_string(dir.join(FILE))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, list: &[Schedule]) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    // Write then rename, so a crash mid-write never loses every schedule.
    let tmp = dir.join(format!("{FILE}.tmp"));
    fs::write(&tmp, json).map_err(|e| e.to_string())?;
    fs::rename(&tmp, dir.join(FILE)).map_err(|e| e.to_string())
}

/// Add a new schedule (id 0) or replace the one with the same id.
/// Returns the stored copy.
pub fn upsert(list: &mut Vec<Schedule>, mut item: Schedule, now: i64) -> Result<Schedule, String> {
    item.validate()?;
    item.prompt = item.prompt.trim().to_string();
    item.name = item.name.trim().to_string();
    if let When::Repeat { days, .. } = &mut item.when {
        days.sort_unstable();
        days.dedup();
        if days.len() == 7 {
            days.clear();
        }
    }
    if let Some(existing) = list.iter_mut().find(|s| s.id == item.id && item.id != 0) {
        // Changing the time of a finished one-off sets it up again.
        if existing.when != item.when {
            item.last_run = None;
            item.created = now;
        } else {
            item.last_run = existing.last_run;
            item.created = existing.created;
        }
        *existing = item.clone();
        return Ok(item);
    }
    if let Some(slot) = item.once_slot() {
        if slot < now {
            return Err("That time has already passed.".into());
        }
    }
    item.id = list.iter().map(|s| s.id).max().unwrap_or(0) + 1;
    item.created = now;
    item.last_run = None;
    list.push(item.clone());
    Ok(item)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: i64, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        stamp(y, mo, d, h, mi)
    }

    fn weekdays_at_nine() -> Schedule {
        Schedule {
            id: 1,
            prompt: "open my calendar and summarise today".into(),
            when: When::Repeat {
                time: "09:00".into(),
                days: vec![0, 1, 2, 3, 4],
            },
            created: at(2026, 11, 1, 12, 0),
            ..Default::default()
        }
    }

    #[test]
    fn calendar_round_trips() {
        for days in [-1000, 0, 1, 59, 365, 20_000, 20_411] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(days_from_civil(2028, 2, 29)), (2028, 2, 29));
        // 19 November 2026 is a Thursday.
        assert_eq!(weekday(at(2026, 11, 19, 10, 0)), 3);
        assert_eq!(weekday(at(2026, 11, 23, 0, 0)), 0);
        assert_eq!(format_stamp(at(2026, 11, 24, 9, 5)), "Tue 24 Nov, 09:05");
    }

    #[test]
    fn times_and_dates_are_checked() {
        assert_eq!(parse_time("9:30"), Some((9, 30)));
        assert_eq!(parse_time("24:00"), None);
        assert_eq!(parse_time("nine"), None);
        assert_eq!(parse_date("2026-02-29"), None);
        assert_eq!(parse_date("2028-02-29"), Some((2028, 2, 29)));
        assert_eq!(parse_date("26-1-1"), None);
    }

    #[test]
    fn day_words() {
        let s = |v: &[&str]| parse_days(&v.iter().map(|x| x.to_string()).collect::<Vec<_>>());
        assert_eq!(s(&["weekdays"]).unwrap(), vec![0, 1, 2, 3, 4]);
        assert_eq!(s(&["Monday", "wed", "MON"]).unwrap(), vec![0, 2]);
        assert_eq!(s(&["daily"]).unwrap(), Vec::<u8>::new());
        assert_eq!(s(&["weekdays", "weekends"]).unwrap(), Vec::<u8>::new());
        assert!(s(&["someday"]).is_err());
    }

    #[test]
    fn weekday_schedule_fires_once_at_its_time() {
        let mut s = weekdays_at_nine();
        // Thursday 19 Nov.
        assert_eq!(s.due(at(2026, 11, 19, 8, 59)), None);
        let slot = s.due(at(2026, 11, 19, 9, 0)).expect("due at nine");
        assert_eq!(slot, at(2026, 11, 19, 9, 0));
        s.last_run = Some(slot);
        assert_eq!(s.due(at(2026, 11, 19, 9, 1)), None, "not twice");
        assert_eq!(s.due(at(2026, 11, 19, 17, 0)), None);
        // Saturday and Sunday are skipped; Monday runs.
        assert_eq!(s.due(at(2026, 11, 21, 9, 0)), None);
        assert_eq!(s.due(at(2026, 11, 22, 9, 0)), None);
        assert!(s.due(at(2026, 11, 23, 9, 0)).is_some());
    }

    #[test]
    fn missed_runs_catch_up_only_briefly() {
        let s = weekdays_at_nine();
        assert!(
            s.due(at(2026, 11, 19, 9, 25)).is_some(),
            "woke up 25 min late"
        );
        assert_eq!(
            s.due(at(2026, 11, 19, 10, 0)),
            None,
            "too late: wait for tomorrow"
        );
        let mut paused = weekdays_at_nine();
        paused.paused = true;
        assert_eq!(paused.due(at(2026, 11, 19, 9, 0)), None);
    }

    #[test]
    fn a_new_schedule_does_not_catch_up_on_the_past() {
        let mut s = weekdays_at_nine();
        s.created = at(2026, 11, 19, 9, 10);
        assert_eq!(s.due(at(2026, 11, 19, 9, 12)), None);
        assert!(s.due(at(2026, 11, 20, 9, 0)).is_some());
    }

    #[test]
    fn late_evening_schedule_noticed_after_midnight() {
        let s = Schedule {
            when: When::Repeat {
                time: "23:50".into(),
                days: vec![],
            },
            prompt: "x".into(),
            ..Default::default()
        };
        assert_eq!(
            s.due(at(2026, 11, 20, 0, 5)),
            Some(at(2026, 11, 19, 23, 50))
        );
    }

    #[test]
    fn one_off_reminder() {
        let mut list = Vec::new();
        let now = at(2026, 11, 19, 14, 0);
        let reminder = Schedule {
            prompt: "remind me to send the report".into(),
            when: When::Once {
                date: "2026-11-19".into(),
                time: "17:00".into(),
            },
            ..Default::default()
        };
        let mut stored = upsert(&mut list, reminder.clone(), now).unwrap();
        assert_eq!(stored.id, 1);
        assert_eq!(stored.next_run(now), Some(at(2026, 11, 19, 17, 0)));
        assert_eq!(stored.due(at(2026, 11, 19, 16, 59)), None);
        // Even if the computer was off at five, it still reminds.
        let slot = stored.due(at(2026, 11, 19, 19, 30)).unwrap();
        stored.last_run = Some(slot);
        assert!(stored.finished());
        assert_eq!(stored.due(at(2026, 11, 19, 19, 31)), None);
        assert_eq!(stored.next_run(at(2026, 11, 19, 19, 31)), None);

        let past = Schedule {
            when: When::Once {
                date: "2026-11-19".into(),
                time: "09:00".into(),
            },
            ..reminder
        };
        assert!(upsert(&mut list, past, now).is_err());
    }

    #[test]
    fn next_run_skips_to_the_right_day() {
        let s = weekdays_at_nine();
        // Friday after nine: next is Monday.
        assert_eq!(
            s.next_run(at(2026, 11, 20, 9, 30)),
            Some(at(2026, 11, 23, 9, 0))
        );
        assert_eq!(
            s.next_run(at(2026, 11, 20, 8, 0)),
            Some(at(2026, 11, 20, 9, 0))
        );
    }

    #[test]
    fn editing_keeps_identity_and_resets_a_changed_time() {
        let now = at(2026, 11, 19, 8, 0);
        let mut list = Vec::new();
        let first = upsert(&mut list, weekdays_at_nine(), now).unwrap();
        list[0].last_run = Some(at(2026, 11, 19, 9, 0));
        // Renaming keeps the history.
        let edited = Schedule {
            name: "Morning brief".into(),
            last_run: None, // the UI does not send history back
            ..list[0].clone()
        };
        let renamed = upsert(&mut list, edited, now).unwrap();
        assert_eq!(renamed.id, first.id);
        assert_eq!(renamed.last_run, Some(at(2026, 11, 19, 9, 0)));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].label(), "Morning brief");
        // A second one gets a new id.
        let new = Schedule {
            id: 0,
            ..weekdays_at_nine()
        };
        let other = upsert(&mut list, new, now).unwrap();
        assert_eq!(other.id, 2);
        assert!(upsert(
            &mut list,
            Schedule {
                prompt: " ".into(),
                ..Default::default()
            },
            now
        )
        .is_err());
    }

    #[test]
    fn descriptions() {
        assert_eq!(weekdays_at_nine().when.describe(), "every weekday at 09:00");
        assert_eq!(
            When::Repeat {
                time: "07:30".into(),
                days: vec![0, 2]
            }
            .describe(),
            "every Mon, Wed at 07:30"
        );
        assert_eq!(
            When::Once {
                date: "2026-11-19".into(),
                time: "17:00".into()
            }
            .describe(),
            "once, Thu 19 Nov, 17:00"
        );
    }

    #[test]
    fn storage_round_trip() {
        let dir = std::env::temp_dir().join(format!("agent-desk-sched-{}", std::process::id()));
        let list = vec![weekdays_at_nine()];
        save(&dir, &list).unwrap();
        assert_eq!(load(&dir), list);
        let _ = fs::remove_dir_all(dir);
        assert!(load(Path::new("/definitely/not/here")).is_empty());
    }
}
