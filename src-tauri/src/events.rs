//! Events Rust sends to the UI (`AppEvents` in src/shared/types.ts). Each event is a type here that
//! fixes its name and payload together, and every emit goes through [`emit`]. The serde fixture
//! (model.rs, contract_tests) writes a value of each type, which tsc checks against `AppEvents`.

use crate::model::{Account, AppNotification, UsageSnapshot};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::{AppHandle, Emitter};

/// A UI event: its `AppEvents` key, with the type itself as the payload.
pub trait AppEvent: Serialize {
    const NAME: &'static str;
}

/// Send `event` to the UI. The only emit in the app, so every payload is one the fixture checks.
pub fn emit<E: AppEvent>(app: &AppHandle, event: &E) {
    let _ = app.emit(E::NAME, event);
}

/// Latest usage by account id.
#[derive(Serialize)]
#[serde(transparent)]
pub struct UsageUpdated<'a>(pub &'a HashMap<String, UsageSnapshot>);

#[derive(Serialize)]
#[serde(transparent)]
pub struct AccountsChanged<'a>(pub &'a [Account]);

#[derive(Serialize)]
#[serde(transparent)]
pub struct Notification<'a>(pub &'a AppNotification);

/// UTF-8 text from a terminal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalOutput {
    pub id: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalExit {
    pub id: String,
    pub code: Option<i32>,
}

/// Ask the UI to show a screen, e.g. "terminals" from the tray menu.
#[derive(Serialize)]
#[serde(transparent)]
pub struct Navigate<'a>(pub &'a str);

/// The only place events get a name, so `NAMES` lists them all.
macro_rules! events {
    ($($t:ty => $name:literal),+ $(,)?) => {
        $(impl AppEvent for $t { const NAME: &'static str = $name; })+
        /// Every event name, which the fixture checks against `keyof AppEvents`.
        #[cfg(test)]
        pub const NAMES: &[&str] = &[$($name),+];
    };
}

events! {
    UsageUpdated<'_> => "usage-updated",
    AccountsChanged<'_> => "accounts-changed",
    Notification<'_> => "notification",
    TerminalOutput => "terminal-output",
    TerminalExit => "terminal-exit",
    Navigate<'_> => "navigate",
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AccountStatus, NotificationKind, ProviderId, SharingOptions, UsageSource};
    use serde_json::json;
    use std::path::Path;

    /// Each event sends the same JSON as the bare value the emit sites sent before.
    #[test]
    fn payloads_serialize_like_the_bare_values() {
        let snapshot = UsageSnapshot {
            account_id: "a".into(),
            provider_id: ProviderId::Claude,
            fetched_at: "2026-01-01T00:00:00Z".into(),
            source: UsageSource::Api,
            windows: vec![],
            plan: None,
            email: None,
            credits: None,
            error: None,
            error_kind: None,
            stale: None,
        };
        let latest = HashMap::from([("a".to_string(), snapshot)]);
        assert_eq!(serde_json::to_value(UsageUpdated(&latest)).unwrap(), serde_json::to_value(&latest).unwrap());

        let account = Account {
            id: "a".into(),
            provider_id: ProviderId::Codex,
            label: "Work".into(),
            email: None,
            plan: None,
            home_dir: "/h/a".into(),
            is_default_home: false,
            shared_from: None,
            sharing: SharingOptions::default(),
            color: "#000000".into(),
            tags: vec![],
            notes: None,
            status: AccountStatus::Ok,
            status_message: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            last_launched_at: None,
            launch_count: 0,
            monthly_cost: None,
            pinned: None,
        };
        let accounts = vec![account];
        assert_eq!(serde_json::to_value(AccountsChanged(&accounts)).unwrap(), serde_json::to_value(&accounts).unwrap());

        let n = AppNotification {
            id: "n".into(),
            kind: NotificationKind::Info,
            account_id: None,
            title: "t".into(),
            body: "b".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            read: false,
        };
        assert_eq!(serde_json::to_value(Notification(&n)).unwrap(), serde_json::to_value(&n).unwrap());
        assert_eq!(serde_json::to_value(Navigate("terminals")).unwrap(), json!("terminals"));

        let out = TerminalOutput { id: "t-1".into(), data: "hi".into() };
        assert_eq!(serde_json::to_value(out).unwrap(), json!({ "id": "t-1", "data": "hi" }));
        for code in [None, Some(0), Some(-1)] {
            assert_eq!(serde_json::to_value(TerminalExit { id: "t-1".into(), code }).unwrap(), json!({ "id": "t-1", "code": code }));
        }
    }

    /// Emit calls other than `events::emit`, `Emitter` uses and `AppEvent` impls in `src`, as
    /// "name (line n)". Comments are skipped, and a call may be split over lines.
    fn raw_emits(src: &str) -> Vec<String> {
        const CALLS: &[&str] = &["emit", "emit_to", "emit_filter", "emit_str", "emit_str_to", "emit_str_filter"];
        let code: String = src.lines().map(|l| l.split("//").next().unwrap_or_default()).collect::<Vec<_>>().join("\n");
        let is_ident = |c: char| c.is_alphanumeric() || c == '_';
        let mut found = Vec::new();
        let mut start = None;
        for (i, c) in code.char_indices().chain([(code.len(), ' ')]) {
            match (start, is_ident(c)) {
                (None, true) => start = Some(i),
                (Some(s), false) => {
                    start = None;
                    let ident = &code[s..i];
                    let after = code[i..].trim_start();
                    let helper = code[..s].ends_with("events::");
                    let call = CALLS.contains(&ident) && after.starts_with(['(', ':']) && !helper;
                    let event_impl = ident == "AppEvent" && after.strip_prefix("for").is_some_and(|r| r.starts_with(char::is_whitespace));
                    if call || event_impl || ident == "Emitter" {
                        found.push(format!("{ident} (line {})", code[..s].matches('\n').count() + 1));
                    }
                }
                _ => {}
            }
        }
        found
    }

    #[test]
    fn finds_every_raw_emit_form() {
        let src = "app.emit(\n    \"x\", 1);\n\
                   Emitter::emit(&app, \"y\", 2);\n\
                   w.emit_to (\"main\", \"z\", 3);\n\
                   w.emit_filter::<()>(\"a\", 1, |_| true);\n\
                   emit_str(\"b\", s);\n\
                   // app.emit(\"gone\")\n\
                   impl super::AppEvent for X {}\n\
                   emit_accounts(app, state);\n\
                   let emit = 1;\n\
                   fn event<E: AppEvent>(e: &E) {}\n\
                   events::emit(app, &Navigate(\"x\"));\n";
        let want = ["emit (line 1)", "Emitter (line 3)", "emit (line 3)", "emit_to (line 4)", "emit_filter (line 5)", "emit_str (line 6)", "AppEvent (line 8)"];
        assert_eq!(raw_emits(src), want);
    }

    /// Everything else emits through `emit`, and events are only declared in the `events!` list.
    #[test]
    fn only_this_module_emits() {
        fn walk(dir: &Path, skip: &Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, skip, out);
                } else if path.extension().is_some_and(|e| e == "rs") && path != skip {
                    let src = std::fs::read_to_string(&path).unwrap();
                    out.extend(raw_emits(&src).into_iter().map(|hit| format!("{}: {hit}", path.display())));
                }
            }
        }
        let src = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
        let mut found = Vec::new();
        walk(src, &src.join("events.rs"), &mut found);
        assert!(found.is_empty(), "emit through events::emit and declare events in events!:\n{}", found.join("\n"));
        // Here, the macro holds the one impl (split so this line doesn't count).
        assert_eq!(include_str!("events.rs").matches(concat!("impl ", "AppEvent for")).count(), 1);
    }
}
