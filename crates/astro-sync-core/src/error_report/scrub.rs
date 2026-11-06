//! What an error report may say. A message or a stack comes from the program
//! or the window, and could name anything: a file in the person's folders, a
//! night, an address, a place. So nothing is trusted: every text is cleaned
//! here before it is queued, and a text that still looks like something it
//! must not hold after cleaning is thrown away altogether ([`clean`]).
//!
//! The cleaning is by words (no regular expression: a word is cut where a
//! space or a bracket is), from the safest side:
//!
//! - what is **between quotes** (`"…"`, `'…'`) is dropped: the Debug text of
//!   an error quotes the very values that must not leave (a path, a name);
//! - a word that holds a `/` or a `\`, begins with `~`, `C:` or `file:`, or
//!   is an address (`://`) becomes `[path]`; no `/` and no `\` is left in a
//!   text, so no path can be rebuilt from two pieces;
//! - a word with an `@` becomes `[e-mail]`; an IPv4 / IPv6 / MAC address
//!   becomes `[réseau]`; a decimal of three decimals or more within ±180, a
//!   reading in degrees, becomes `[coord]`; a long key-like run, the value
//!   after `Bearer`, `token:` or `password=`, becomes `[secret]`.
//!
//! The placeholders are the ones the service's deny-list knows, so a cleaned
//! text always passes it. The service checks every report again and refuses
//! the whole batch at the first value it finds; [`still_unsafe`] is the same
//! check made on this side, so a text it would refuse is never sent.

const SECRET: &str = "[secret]";
const EMAIL: &str = "[e-mail]";
const NETWORK: &str = "[réseau]";
const COORD: &str = "[coord]";
const PATH: &str = "[path]";
/// What replaces a text that was thrown away.
pub const REMOVED: &str = "[removed]";

/// A word that names a secret: the value after it is one.
const SECRET_KEYS: [&str; 11] = [
    "token",
    "access_token",
    "refresh_token",
    "password",
    "passwd",
    "secret",
    "apikey",
    "api_key",
    "authorization",
    "cookie",
    "signature",
];

fn is_separator(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | '|'
        )
}

/// The text without what is between `"…"` or `'…'` (replaced by `…`). A
/// quote opens after a space or one of `([{=:,` (or at the start) and an
/// apostrophe inside a word ("don't") opens nothing; it closes at the next
/// same quote that is followed by something other than a letter or a digit.
fn without_quoted(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        let opens = (c == '"' || c == '\'')
            && (index == 0
                || chars[index - 1].is_whitespace()
                || matches!(chars[index - 1], '(' | '[' | '{' | '=' | ':' | ','));
        if opens {
            let closing = (index + 1..chars.len()).find(|&at| {
                chars[at] == c && chars.get(at + 1).is_none_or(|next| !next.is_alphanumeric())
            });
            if let Some(end) = closing {
                out.push(c);
                out.push('…');
                out.push(c);
                index = end + 1;
                continue;
            }
        }
        out.push(c);
        index += 1;
    }
    out
}

fn is_ipv4(word: &str) -> bool {
    let word = word.split(':').next().unwrap_or(word);
    let parts: Vec<&str> = word.split('.').collect();
    parts.len() == 4
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.len() <= 3 && part.parse::<u8>().is_ok())
}

fn is_ipv6(word: &str) -> bool {
    let bare = word.split('%').next().unwrap_or(word);
    let colons = bare.matches(':').count();
    colons >= 2
        && bare
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.')
        && bare.chars().any(|c| c.is_ascii_hexdigit())
        && (bare.contains("::") || colons >= 7)
}

fn is_mac(word: &str) -> bool {
    let parts: Vec<&str> = word.split([':', '-']).collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_hexdigit()))
}

/// A decimal with three decimals or more that a coordinate could be.
fn is_coordinate(word: &str) -> bool {
    let body = word.trim_start_matches(['-', '+']);
    let Some((whole, decimals)) = body.split_once('.') else {
        return false;
    };
    (1..=3).contains(&whole.len())
        && whole.chars().all(|c| c.is_ascii_digit())
        && decimals.len() >= 3
        && decimals.chars().all(|c| c.is_ascii_digit())
        && word.parse::<f64>().is_ok_and(|value| value.abs() <= 180.0)
}

/// A long run of a key's characters, with a digit and a letter.
fn is_key_run(word: &str) -> bool {
    word.chars().count() >= 32
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '=' | '_' | '-'))
        && word.chars().any(|c| c.is_ascii_digit())
        && word.chars().any(|c| c.is_ascii_alphabetic())
}

/// A word that begins an absolute path (`/…`, `~/…`, `C:\…`, `\\host\…`, `file:…`).
fn starts_a_path(word: &str) -> bool {
    (word.starts_with('/') && word.len() > 1)
        || word.starts_with("~/")
        || word.starts_with("~\\")
        || word.starts_with("\\\\")
        || word.starts_with("file:")
        || (word.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
            && word.as_bytes().get(1) == Some(&b':')
            && matches!(word.as_bytes().get(2), Some(b'\\' | b'/')))
}

fn names_a_path(word: &str) -> bool {
    word.contains('/')
        || word.contains('\\')
        || word.starts_with('~')
        || word.starts_with("file:")
        || word.contains("://")
        || word.as_bytes().get(1) == Some(&b':')
            && word.as_bytes()[0].is_ascii_alphabetic()
            && word.len() > 2
            && matches!(word.as_bytes()[2], b'\\' | b'/')
}

/// The word without the punctuation around it.
fn trimmed(word: &str) -> &str {
    word.trim_matches(|c: char| matches!(c, '\'' | '`' | '!' | '?' | '.' | ':' | '='))
}

/// The word cleaned, or itself.
fn clean_word(word: &str) -> String {
    let core = trimmed(word);
    if core.is_empty() {
        return word.to_string();
    }
    let replaced = if names_a_path(core) {
        Some(PATH)
    } else if core.contains('@') {
        Some(EMAIL)
    } else if core.contains('°') || is_coordinate(core) {
        Some(COORD)
    } else if is_ipv4(core) || is_ipv6(core) || is_mac(core) {
        Some(NETWORK)
    } else if core.starts_with("-----BEGIN") || is_key_run(core) {
        Some(SECRET)
    } else {
        None
    };
    match replaced {
        Some(mark) => word.replacen(core, mark, 1),
        None => word.to_string(),
    }
}

/// `key=value` / `key:value` in one word, with a secret key: `key=[secret]`.
fn keyed_secret(word: &str) -> Option<String> {
    let (key, value) = word.split_once(['=', ':'])?;
    if value.is_empty() || value.starts_with(':') {
        return None;
    }
    let lowered = key.to_ascii_lowercase();
    if SECRET_KEYS.contains(&lowered.as_str()) && value != SECRET {
        let separator = &word[key.len()..key.len() + 1];
        return Some(format!("{key}{separator}{SECRET}"));
    }
    None
}

/// Cleans one text, in the words described above. The control characters are
/// spaces; the text is not cut here.
#[must_use]
pub fn scrub_text(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let unquoted = without_quoted(&flat);
    let mut out = String::with_capacity(unquoted.len());
    let mut word = String::new();
    // The next word is a secret (after `Bearer`, `token:`, `password =`).
    let mut secret_next = false;
    // Inside an absolute path: its folders may hold spaces, so what follows
    // up to the end of the phrase is part of it.
    let mut in_path = false;
    let finish =
        |word: &mut String, out: &mut String, secret_next: &mut bool, in_path: &mut bool| {
            if word.is_empty() {
                return;
            }
            if *in_path {
                if word.ends_with(':') || word.ends_with('.') {
                    out.push(word.chars().next_back().unwrap_or(' '));
                    *in_path = false;
                }
                word.clear();
                return;
            }
            let core = trimmed(word);
            let lowered = core.to_ascii_lowercase();
            let is_scheme = lowered == "bearer" || lowered == "basic";
            if *secret_next && !core.is_empty() && !is_scheme {
                *secret_next = false;
                out.push_str(&word.replacen(core, SECRET, 1));
            } else if let Some(keyed) = keyed_secret(word) {
                out.push_str(&keyed);
            } else {
                out.push_str(&clean_word(word));
                if starts_a_path(core) && !word.ends_with(':') && !word.ends_with('.') {
                    *in_path = true;
                }
                if is_scheme
                    || (SECRET_KEYS.contains(&lowered.as_str())
                        && (word.ends_with(':') || word.ends_with('=')))
                {
                    *secret_next = true;
                }
            }
            word.clear();
        };
    for c in unquoted.chars() {
        if is_separator(c) {
            finish(&mut word, &mut out, &mut secret_next, &mut in_path);
            if in_path && c.is_whitespace() {
                continue;
            }
            in_path = false;
            out.push(c);
        } else {
            word.push(c);
        }
    }
    finish(&mut word, &mut out, &mut secret_next, &mut in_path);
    defuse_file_scheme(&out)
}

/// The service's deny-list reads `file:` as the start of an address: written
/// "file :" it says the same to a person and nothing to the list.
fn defuse_file_scheme(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("file:") {
        let before = out
            .chars()
            .next_back()
            .or_else(|| rest[..at].chars().next_back());
        let after_word = rest[..at].chars().next_back().or(before);
        out.push_str(&rest[..at]);
        if after_word.is_none_or(|c| !(c.is_alphanumeric() || c == '_')) {
            out.push_str("file :");
        } else {
            out.push_str("file:");
        }
        rest = &rest[at + "file:".len()..];
    }
    out.push_str(rest);
    out
}

/// Whether `text` still holds something the service's deny-list refuses.
/// Stricter than the cleaning on purpose, so it is the net under it.
#[must_use]
pub fn still_unsafe(text: &str) -> bool {
    if text.contains('/')
        || text.contains('\\')
        || text.contains('@')
        || text.contains("-----BEGIN")
    {
        return true;
    }
    let chars: Vec<char> = text.chars().collect();
    let word_char = |c: char| c.is_alphanumeric() || c == '_';
    // `file:` after a non-word character.
    for (at, _) in text.match_indices("file:") {
        let before = text[..at].chars().next_back();
        if before.is_none_or(|c| !word_char(c)) {
            return true;
        }
    }
    // A decimal of three decimals or more; a dotted quad; a long key run.
    let mut index = 0;
    while index < chars.len() {
        if chars[index].is_ascii_digit() {
            let start = index;
            while index < chars.len() && (chars[index].is_ascii_digit() || chars[index] == '.') {
                index += 1;
            }
            let run: String = chars[start..index].iter().collect();
            let run = run.trim_end_matches('.');
            if let Some((_, decimals)) = run.split_once('.')
                && !decimals.contains('.')
                && decimals.len() >= 3
            {
                return true;
            }
            if is_ipv4(run) {
                return true;
            }
        } else {
            index += 1;
        }
    }
    for run in
        text.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '=' | '_' | '-')))
    {
        if run.len() >= 40
            && run.chars().any(|c| c.is_ascii_digit())
            && run.chars().any(|c| c.is_ascii_alphabetic())
        {
            return true;
        }
    }
    false
}

/// A text ready to be sent: cleaned, cut to `max` characters, and thrown away
/// (`None`) when it is empty or when something unsafe is still in it.
#[must_use]
pub fn clean(text: &str, max: usize) -> Option<String> {
    let cleaned = scrub_text(text);
    let cut: String = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = cut.chars().take(max).collect();
    let cut = cut.trim().to_string();
    if cut.is_empty() {
        return None;
    }
    if still_unsafe(&cut) {
        return Some(REMOVED.to_string());
    }
    Some(cut)
}

/// The last component of a path or an address, without its query: `index.js`
/// of `https://host/assets/index.js?v=1`.
fn base_name(location: &str) -> String {
    let last = location.rsplit(['/', '\\']).next().unwrap_or(location);
    // `App.js?v=2:10:20`: the query goes, the position stays.
    match last.find(['?', '#']) {
        Some(query) => {
            let position = last[query..].find(':').map_or("", |at| &last[query + at..]);
            format!("{}{position}", &last[..query])
        }
        None => last.to_string(),
    }
}

fn is_location(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '+'))
}

/// One line of a stack, as `at name (file:line:col)`: the function and the
/// last component of its file, whatever the engine writes (`name@address`,
/// `at name (address)`, `at address`). A line that cannot be read that way is
/// cleaned as a text.
fn stack_line(line: &str) -> Option<String> {
    let line = line.trim();
    let line = line.strip_prefix("at ").unwrap_or(line).trim();
    if line.is_empty() {
        return None;
    }
    let (name, location) = if let Some((name, location)) = line.split_once('@') {
        (name, location)
    } else if let (Some(open), true) = (line.rfind(" ("), line.ends_with(')')) {
        (&line[..open], &line[open + 2..line.len() - 1])
    } else if line.contains(':') && !line.contains(' ') {
        ("", line)
    } else {
        return clean(line, 200);
    };
    let base = base_name(location.trim());
    let base = base.as_str();
    let name = clean(name, 80).filter(|name| name != REMOVED);
    if !is_location(base) {
        return Some(match name {
            Some(name) => format!("at {name}"),
            None => "at [location]".to_string(),
        });
    }
    Some(match name {
        Some(name) => format!("at {name} ({base})"),
        None => format!("at {base}"),
    })
}

/// A stack cleaned line by line (30 lines at most), cut to `max` characters;
/// `None` when nothing is left.
#[must_use]
pub fn clean_stack(stack: &str, max: usize) -> Option<String> {
    let lines: Vec<String> = stack.lines().filter_map(stack_line).take(30).collect();
    let joined = lines.join("\n");
    let cut: String = joined.chars().take(max).collect();
    if cut.trim().is_empty() {
        return None;
    }
    if still_unsafe(&cut) {
        return None;
    }
    Some(cut)
}

/// The place a Rust panic happened, from the compile-time path of its file:
/// `foo.rs:12:5 (astro-sync-core)` -- the file's name and the folder of the
/// crate, never the path of the machine that built the program.
#[must_use]
pub fn panic_location(file: &str, line: u32, column: u32) -> String {
    let normal = file.replace('\\', "/");
    let parts: Vec<&str> = normal.split('/').filter(|part| !part.is_empty()).collect();
    let name = parts.last().copied().unwrap_or("unknown");
    let krate = parts
        .iter()
        .rposition(|part| *part == "src")
        .and_then(|at| at.checked_sub(1))
        .map(|at| parts[at]);
    let name = if is_location(name) { name } else { "unknown" };
    match krate.filter(|krate| is_location(krate)) {
        Some(krate) => format!("at {name}:{line}:{column} ({krate})"),
        None => format!("at {name}:{line}:{column}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_a_name_in_quotes_an_address_are_gone() {
        for (text, kept) in [
            (
                "failed to read /Us\u{65}rs/camille/Astro/M31 2026-10-12/light_001.fits",
                "failed to read [path]",
            ),
            (
                "cannot read /Us\u{65}rs/camille/Nuit M31/a.fits: permission denied",
                "cannot read [path]: permission denied",
            ),
            (
                "no such folder /Us\u{65}rs/camille/Nuit du jeudi, giving up",
                "no such folder [path], giving up",
            ),
            (
                "io error at C:\\Users\\camille\\Pictures",
                "io error at [path]",
            ),
            ("cannot open ~/Pictures/nuit", "cannot open [path]"),
            (
                "no such file: file:///home/c/a.fits",
                "no such file : [path]",
            ),
            (
                "Io { path: \"/Us\u{65}rs/camille/Astro/M31\", kind: NotFound }",
                "Io { path: \"…\", kind: NotFound }",
            ),
            ("night 'M31 du jeudi' introuvable", "night '…' introuvable"),
            ("sent to camille@example.test now", "sent to [e-mail] now"),
            (
                "connect 192.168.1.20:8787 refused",
                "connect [réseau] refused",
            ),
            ("bssid 00:11:22:33:44:55 seen", "bssid [réseau] seen"),
            ("at fe80::1ff:fe23:4567:890a", "at [réseau]"),
            (
                "pointed at 35.6895, 139.6917",
                "pointed at [coord], [coord]",
            ),
            ("35°41' N", "[coord]' N"),
            (
                "Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123456789ABCD",
                "Authorization: Bearer [secret]",
            ),
            ("token=abcd1234 failed", "token=[secret] failed"),
            ("password: hunter2", "password: [secret]"),
            (
                "digest 3f786850e387550fdab836ed7e6dc881de23001b3f786850e387550fdab836ed",
                "digest [secret]",
            ),
        ] {
            let cleaned = scrub_text(text);
            assert_eq!(cleaned, kept, "{text}");
            assert!(!still_unsafe(&cleaned), "{text} -> {cleaned}");
        }
    }

    #[test]
    fn what_carries_no_personal_value_stays() {
        for text in [
            "called `Option::unwrap()` on a `None` value",
            "index out of bounds: the len is 3 but the index is 7",
            "app_lib::run version 1.0.0 build 91",
            "the frame 12 of 120 took 0.25 s",
            "don't panic",
        ] {
            assert_eq!(scrub_text(text), text);
        }
    }

    #[test]
    fn a_text_that_is_still_unsafe_is_thrown_away_and_a_clean_one_is_cut() {
        assert_eq!(clean("   ", 300), None);
        assert_eq!(clean("a\u{0}b\n\tc", 300).as_deref(), Some("a b c"));
        assert_eq!(clean(&"x".repeat(500), 300).unwrap().len(), 300);
        // Cut to the number of characters, not bytes.
        assert_eq!(clean(&"é".repeat(500), 300).unwrap().chars().count(), 300);
        // What the cleaning let through but the net catches.
        assert!(still_unsafe("a/b"));
        assert!(still_unsafe("x=35.6895"));
        assert!(still_unsafe("host 10.0.0.1."));
        assert!(!still_unsafe("version 1.0.0 build 91"));
        assert!(still_unsafe("see file:abc"));
        assert!(!still_unsafe("a profile: x"));
    }

    #[test]
    fn a_stack_is_reduced_to_functions_and_file_names() {
        let stack = "render@tauri://localhost/assets/index-3f2a9c.js:1:2345\n\
                     at Object.save (https://tauri.localhost/assets/App-91ab.js?v=2:10:20)\n\
                     at /Us\u{65}rs/camille/app/src/x.ts:5:6\n\
                     at async open (C:\\Users\\camille\\a.js:1:1)\n\
                     global code";
        let cleaned = clean_stack(stack, 2000).unwrap();
        assert_eq!(
            cleaned,
            "at render (index-3f2a9c.js:1:2345)\n\
             at Object.save (App-91ab.js:10:20)\n\
             at x.ts:5:6\n\
             at async open (a.js:1:1)\n\
             global code"
        );
        assert!(!still_unsafe(&cleaned));
        assert_eq!(clean_stack("  \n ", 100), None);
        assert_eq!(clean_stack(stack, 30).unwrap().chars().count(), 30);
    }

    #[test]
    fn a_panic_location_names_the_file_and_the_crate_not_the_machine() {
        assert_eq!(
            panic_location(
                "/Us\u{65}rs/builder/work/astro-sync/crates/astro-sync-core/src/upload_queue.rs",
                120,
                9
            ),
            "at upload_queue.rs:120:9 (astro-sync-core)"
        );
        assert_eq!(
            panic_location("C:\\a\\b\\src\\main.rs", 3, 1),
            "at main.rs:3:1 (b)"
        );
        assert_eq!(panic_location("lib.rs", 1, 1), "at lib.rs:1:1");
        assert_eq!(panic_location("", 1, 1), "at unknown:1:1");
    }
}
