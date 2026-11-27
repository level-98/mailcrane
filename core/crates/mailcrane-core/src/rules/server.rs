//! Server-side rules through ManageSieve (M4-23, ADR 0004).
//!
//! Mailcrane keeps one script named [`SCRIPT_NAME`] on the account's
//! ManageSieve server, compiled from the account's enabled rules, so they
//! run when the phone is off. A rule the server can't express stays on the
//! device. Mailcrane never overwrites, deletes or deactivates another
//! script: if another one is active, the rules stay local until the user
//! explicitly asks to switch ([`Store::request_server_rules_switch`]).
//!
//! While the script is active, rules in it are not applied again locally
//! ([`on_server`]); a rule edited since the last upload runs locally until
//! the new script is up. A failed upload leaves the server's old script in
//! place and changes nothing locally.

use crate::account::autoconfig::dns::{parse_srv, RecordType};
use crate::account::autoconfig::DnsResolver;
pub use mailcrane_sieve::script::Extensions;
use mailcrane_sieve::script::{self, Field, Flag, Moving, ScriptRule, Test};
pub use mailcrane_sieve::{Secret, SieveError};
use mailcrane_sieve::{Session, SieveConfig, DEFAULT_PORT};
use mailcrane_store::{
    AccountId, FolderRole, NewRule, RuleAction, RuleCondition, RuleId, ServerRules,
    ServerRulesState, Store, StoreError,
};
use std::collections::{HashMap, HashSet};

/// The only script Mailcrane writes.
pub const SCRIPT_NAME: &str = "mailcrane";

/// Most rules put in the script; later ones stay on the device.
pub const MAX_SERVER_RULES: usize = 200;

/// How long "no ManageSieve server" is believed before checking again.
pub const RECHECK_UNAVAILABLE_SECS: i64 = 3 * 86_400;
/// How often an account with a server is checked again (another client may
/// have changed the active script).
pub const RECHECK_SECS: i64 = 86_400;
/// After a failed attempt, how long until changed rules are tried again.
pub const RETRY_SECS: i64 = 15 * 60;

/// Which rules go to the server, for one account.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// In order, with each compiled rule's fingerprint.
    pub server: Vec<(RuleId, ScriptRule, String)>,
    /// Enabled rules that stay on the device.
    pub device: Vec<RuleId>,
}

impl Plan {
    /// The script for the server rules.
    pub fn script(&self, ext: &Extensions) -> Result<String, script::ScriptError> {
        let rules: Vec<ScriptRule> = self.server.iter().map(|(_, r, _)| r.clone()).collect();
        script::compile(&rules, ext)
    }

    fn fingerprints(&self) -> Vec<(RuleId, String)> {
        self.server
            .iter()
            .map(|(id, _, f)| (*id, f.clone()))
            .collect()
    }
}

/// FNV-1a, 64 bits, as hex: a change detector, not a security measure.
fn fnv(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// The account's folder names for `fileinto`: the full name, decoded to
/// UTF-8 (RFC 5228 §4.1 takes UTF-8; the server converts).
struct Folders {
    names: HashMap<i64, String>,
    archive: Option<String>,
    trash: Option<String>,
}

impl Folders {
    fn load(store: &Store, account: AccountId) -> Result<Self, StoreError> {
        let mut names = HashMap::new();
        let mut archive = None;
        let mut trash = None;
        for f in store.list_folders(account)? {
            // A virtual folder (#390) takes no moves: a rule filing into one stays
            // local, where it's skipped.
            if !f.details.selectable || f.details.virtual_role.is_some() {
                continue;
            }
            let name = mailcrane_imap::mutf7::decode(&f.details.remote_name)
                .unwrap_or_else(|_| f.details.remote_name.clone());
            if name.chars().any(char::is_control) {
                continue;
            }
            match f.details.role {
                Some(FolderRole::Archive) => archive = Some(name.clone()),
                Some(FolderRole::Trash) => trash = Some(name.clone()),
                _ => {}
            }
            names.insert(f.id.0, name);
        }
        Ok(Self {
            names,
            archive,
            trash,
        })
    }
}

/// The rule as Sieve, or `None` if only the device can run it.
fn to_script_rule(rule: &NewRule, folders: &Folders) -> Option<ScriptRule> {
    let mut tests = Vec::with_capacity(rule.conditions.len());
    for c in &rule.conditions {
        let (field, text) = match c {
            RuleCondition::From(t) => (Field::From, t),
            RuleCondition::ToCc(t) => (Field::ToCc, t),
            RuleCondition::Subject(t) => (Field::Subject, t),
            RuleCondition::ListId(t) => (Field::ListId, t),
            // Needs the MIME structure, or only applies to existing mail.
            RuleCondition::HasAttachment | RuleCondition::OlderThanDays(_) => return None,
        };
        tests.push(Test {
            field,
            text: text.clone(),
        });
    }
    let mut flags = Vec::new();
    let mut moving: Option<Moving> = None;
    for a in &rule.actions {
        match a {
            RuleAction::MarkRead => flags.push(Flag::Seen),
            RuleAction::Flag => flags.push(Flag::Flagged),
            // The first moving action wins, as locally.
            _ if moving.is_some() => {}
            RuleAction::Move(f) => {
                moving = Some(Moving::FileInto(folders.names.get(&f.0)?.clone()))
            }
            RuleAction::Archive => moving = Some(Moving::FileInto(folders.archive.clone()?)),
            // Locally: to Trash, or deleted for good when there is none.
            RuleAction::Delete => {
                moving = Some(match &folders.trash {
                    Some(t) => Moving::FileInto(t.clone()),
                    None => Moving::Discard,
                });
            }
        }
    }
    Some(ScriptRule {
        match_all: rule.match_all,
        tests,
        flags,
        moving,
        stop: rule.stop_processing,
    })
}

/// Decides, for `account`'s enabled rules in order, which go to the server.
///
/// A rule stays on the device if the server can't express it. After a
/// device rule that moves mail or stops processing, every later rule stays
/// on the device too, so the order the rules run in still holds.
pub fn plan(store: &Store, account: AccountId, ext: &Extensions) -> Result<Plan, StoreError> {
    let folders = Folders::load(store, account)?;
    let mut plan = Plan::default();
    let mut rest_local = false;
    for rule in store.list_rules(Some(account))? {
        if !rule.rule.enabled || rule.rule.conditions.is_empty() || rule.rule.actions.is_empty() {
            continue;
        }
        let compiled = if rest_local || plan.server.len() >= MAX_SERVER_RULES {
            None
        } else {
            to_script_rule(&rule.rule, &folders).filter(|r| r.supported(ext))
        };
        match compiled {
            Some(r) => {
                let fingerprint = fnv(&format!("{r:?}"));
                plan.server.push((rule.id, r, fingerprint));
            }
            None => {
                let moves = rule.rule.actions.iter().any(|a| {
                    matches!(
                        a,
                        RuleAction::Move(_) | RuleAction::Archive | RuleAction::Delete
                    )
                });
                if moves || rule.rule.stop_processing {
                    rest_local = true;
                }
                plan.device.push(rule.id);
            }
        }
    }
    Ok(plan)
}

/// The rules the server runs for `account` right now, so the device
/// doesn't run them again: the ones in the active script whose settings
/// haven't changed since it was uploaded.
pub fn on_server(store: &Store, account: AccountId) -> Result<HashSet<RuleId>, StoreError> {
    let Some(state) = store.server_rules(account)? else {
        return Ok(HashSet::new());
    };
    if state.state != ServerRulesState::Active || state.deployed.is_empty() {
        return Ok(HashSet::new());
    }
    let ext = Extensions::from_list(&state.extensions);
    let now: HashMap<RuleId, String> = plan(store, account, &ext)?
        .server
        .into_iter()
        .map(|(id, _, f)| (id, f))
        .collect();
    Ok(state
        .deployed
        .into_iter()
        .filter(|(id, f)| now.get(id) == Some(f))
        .map(|(id, _)| id)
        .collect())
}

/// How to log in to the account's ManageSieve server.
#[derive(Debug, Clone)]
pub struct SieveLogin {
    pub username: String,
    pub password: Secret,
    pub imap_host: String,
    /// The address's domain, for the `_sieve._tcp` SRV record.
    pub email_domain: String,
    /// Tests only: the testkit's root certificate.
    pub extra_root_cert: Option<Vec<u8>>,
    /// Tests only: the port to use instead of 4190 on the IMAP host.
    pub port_override: Option<u16>,
}

/// What [`sync`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// Checked recently; nothing to do.
    Skipped,
    /// No usable server (cached).
    Unavailable(SieveError),
    /// Another script is active; rules stay on the device.
    OtherActive,
    /// The server already has the current script.
    UpToDate,
    /// The script was uploaded and is active.
    Uploaded,
    /// Something failed; the server and the device are as before.
    Failed(SieveError),
}

fn needs_check(state: Option<&ServerRules>, script_hash: Option<&str>, now: i64) -> bool {
    let Some(s) = state else {
        return true;
    };
    let age = now.saturating_sub(s.checked_at);
    match s.state {
        ServerRulesState::Unavailable => age >= RECHECK_UNAVAILABLE_SECS,
        ServerRulesState::OtherActive => s.switch_requested || age >= RECHECK_SECS,
        ServerRulesState::Active => {
            age >= RECHECK_SECS || (s.script_hash.as_deref() != script_hash && age >= RETRY_SECS)
        }
    }
}

/// Whether an SRV target may receive the account's password: the IMAP host
/// itself, or a host in the address's domain or the IMAP host's parent
/// domain. DNS answers aren't authenticated, so a target anywhere else is
/// ignored (ADR 0004).
fn trusted_target(target: &str, login: &SieveLogin) -> bool {
    let t = target.trim_end_matches('.').to_ascii_lowercase();
    let within = |domain: &str| {
        let d = domain.trim_end_matches('.').to_ascii_lowercase();
        !d.is_empty() && (t == d || t.ends_with(&format!(".{d}")))
    };
    let imap = login.imap_host.to_ascii_lowercase();
    let parent = imap
        .split_once('.')
        .map(|(_, p)| p)
        .filter(|p| p.contains('.'));
    t == imap || within(&login.email_domain) || parent.is_some_and(within)
}

/// Where to look for the server, in order: a trusted `_sieve._tcp` SRV
/// target (RFC 5804 §1.8), then the IMAP host on port 4190.
pub async fn endpoints<R: DnsResolver>(dns: &R, login: &SieveLogin) -> Vec<(String, u16)> {
    let mut out = Vec::new();
    if let Some(port) = login.port_override {
        out.push((login.imap_host.clone(), port));
        return out;
    }
    let name = format!("_sieve._tcp.{}", login.email_domain);
    if let Some(answer) = dns.query(&name, RecordType::Srv).await {
        if let Some(srvs) = parse_srv(&answer) {
            if let Some(s) = srvs
                .into_iter()
                .find(|s| s.port != 0 && !s.target.is_empty() && trusted_target(&s.target, login))
            {
                out.push((s.target.trim_end_matches('.').to_owned(), s.port));
            }
        }
    }
    let default = (login.imap_host.clone(), DEFAULT_PORT);
    if !out
        .iter()
        .any(|(h, p)| h.eq_ignore_ascii_case(&default.0) && *p == default.1)
    {
        out.push(default);
    }
    out
}

async fn connect(
    endpoints: &[(String, u16)],
    login: &SieveLogin,
) -> Result<(Session, String, u16), SieveError> {
    let mut last = SieveError::CannotConnect;
    for (host, port) in endpoints {
        let mut config = SieveConfig::new(host.clone(), *port);
        // Inside a sync: don't hold it up long for a port nobody answers.
        config.connect_timeout = std::time::Duration::from_secs(10);
        if let Some(der) = &login.extra_root_cert {
            config = config.with_extra_root_cert(der.clone());
        }
        match Session::connect(&config, &login.username, &login.password).await {
            Ok(s) => return Ok((s, host.clone(), *port)),
            // A wrong password is the same on every host; don't send it again.
            Err(SieveError::AuthFailed) => return Err(SieveError::AuthFailed),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Brings the account's server script up to date with its rules, when it's
/// time to check. Only a local database failure is an error; anything the
/// server does is reported in the outcome, and leaves the mailbox and the
/// server as they were.
pub async fn sync<R: DnsResolver>(
    store: &Store,
    account: AccountId,
    login: &SieveLogin,
    dns: &R,
    now: i64,
) -> Result<SyncOutcome, StoreError> {
    let state = store.server_rules(account)?;
    let known_ext = state
        .as_ref()
        .map(|s| Extensions::from_list(&s.extensions))
        .unwrap_or_default();
    let known_hash = plan(store, account, &known_ext)?
        .script(&known_ext)
        .ok()
        .map(|s| fnv(&s));
    if !needs_check(state.as_ref(), known_hash.as_deref(), now) {
        // Keep the device's view of what runs where in step with edits that
        // didn't change the script (a rename, say).
        if let Some(mut s) = state.filter(|s| s.state == ServerRulesState::Active) {
            if s.script_hash == known_hash {
                let fresh = plan(store, account, &known_ext)?.fingerprints();
                if fresh != s.deployed {
                    s.deployed = fresh;
                    store.save_server_rules(&s)?;
                }
            }
        }
        return Ok(SyncOutcome::Skipped);
    }

    let targets = endpoints(dns, login).await;
    let (mut session, host, port) = match connect(&targets, login).await {
        Ok(s) => s,
        Err(e) => return record_failure(store, account, state, e, now),
    };
    let outcome = talk(
        store,
        account,
        &mut session,
        state.clone(),
        &host,
        port,
        now,
    )
    .await;
    session.logout().await;
    match outcome {
        Ok(o) => Ok(o),
        Err(TalkError::Store(e)) => Err(e),
        Err(TalkError::Sieve(e)) => record_failure(store, account, state, e, now),
    }
}

enum TalkError {
    Store(StoreError),
    Sieve(SieveError),
}

impl From<StoreError> for TalkError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

impl From<SieveError> for TalkError {
    fn from(e: SieveError) -> Self {
        Self::Sieve(e)
    }
}

async fn talk(
    store: &Store,
    account: AccountId,
    session: &mut Session,
    previous: Option<ServerRules>,
    host: &str,
    port: u16,
    now: i64,
) -> Result<SyncOutcome, TalkError> {
    let extensions = session.capabilities().sieve.clone();
    let ext = Extensions::from_list(&extensions);
    let scripts = session.list_scripts().await?;
    let active = scripts.iter().find(|s| s.active).map(|s| s.name.clone());
    let switch_requested = previous.as_ref().is_some_and(|p| p.switch_requested);
    let mut row = ServerRules {
        account_id: account,
        state: ServerRulesState::Active,
        host: Some(host.to_owned()),
        port: Some(port),
        extensions,
        other_script: None,
        script_hash: None,
        checked_at: now,
        switch_requested: false,
        deployed: Vec::new(),
    };
    if let Some(other) = active.as_deref().filter(|a| *a != SCRIPT_NAME) {
        if !switch_requested {
            // Never touch another active script without the user's say-so.
            row.state = ServerRulesState::OtherActive;
            row.other_script = Some(clean_name(other));
            store.save_server_rules(&row)?;
            return Ok(SyncOutcome::OtherActive);
        }
    }
    let plan = plan(store, account, &ext)?;
    // A script too large to send keeps every rule on the device.
    let script = plan.script(&ext).map_err(|_| SieveError::InvalidInput)?;
    let hash = fnv(&script);
    let ours_active = active.as_deref() == Some(SCRIPT_NAME);
    let ours_exists = scripts.iter().any(|s| s.name == SCRIPT_NAME);
    if plan.server.is_empty() && !ours_exists {
        // Nothing for the server: don't create a script, or touch anything.
        row.state = if active.is_some() {
            row.other_script = active.as_deref().map(clean_name);
            ServerRulesState::OtherActive
        } else {
            ServerRulesState::Active
        };
        row.switch_requested = switch_requested && active.is_some();
        row.script_hash = Some(hash);
        store.save_server_rules(&row)?;
        return Ok(SyncOutcome::UpToDate);
    }
    let up_to_date = ours_active
        && previous
            .as_ref()
            .is_some_and(|p| p.script_hash.as_deref() == Some(hash.as_str()))
        && session.get_script(SCRIPT_NAME).await.ok().as_deref() == Some(script.as_str());
    if !up_to_date {
        // RFC 5804 §2.12 and §2.5 first, so a refusal leaves the old script.
        session.check_script(&script).await?;
        session.have_space(SCRIPT_NAME, script.len()).await?;
        session.put_script(SCRIPT_NAME, &script).await?;
        if !ours_active {
            // Only reached with no script active, or when the user asked to
            // switch: SETACTIVE deactivates the other one (RFC 5804 §2.8).
            session.set_active(SCRIPT_NAME).await?;
        }
    }
    row.script_hash = Some(hash);
    row.deployed = plan.fingerprints();
    store.save_server_rules(&row)?;
    Ok(if up_to_date {
        SyncOutcome::UpToDate
    } else {
        SyncOutcome::Uploaded
    })
}

/// A server-supplied script name, safe to show as one line of plain text.
fn clean_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control() && !matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
        .take(64)
        .collect()
}

fn record_failure(
    store: &Store,
    account: AccountId,
    previous: Option<ServerRules>,
    error: SieveError,
    now: i64,
) -> Result<SyncOutcome, StoreError> {
    tracing::warn!(
        error = error.name(),
        "server rules: ManageSieve step failed"
    );
    match previous {
        // The server may still run the script from before; keep everything
        // and only wait before trying again.
        Some(mut p) if p.state != ServerRulesState::Unavailable => {
            p.checked_at = now;
            store.save_server_rules(&p)?;
            Ok(SyncOutcome::Failed(error))
        }
        // Never set up: remember there's no usable server, so every sync
        // doesn't wait for a connection or send a wrong password again. It
        // is checked again after `RECHECK_UNAVAILABLE_SECS`.
        _ => {
            store.save_server_rules(&ServerRules {
                account_id: account,
                state: ServerRulesState::Unavailable,
                host: None,
                port: None,
                extensions: Vec::new(),
                other_script: None,
                script_hash: None,
                checked_at: now,
                switch_requested: false,
                deployed: Vec::new(),
            })?;
            Ok(SyncOutcome::Unavailable(error))
        }
    }
}

#[cfg(test)]
mod tests;
