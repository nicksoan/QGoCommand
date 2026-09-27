//! Template expansion — the port of `MainViewModel.Expand` and the identical
//! private copy in `ChainedShortcutExecutor.ExpandTemplate`.
//!
//! The C# has three behaviours worth calling out, because they look like bugs
//! and are reproduced deliberately so both builds resolve templates identically:
//!
//! * the parameter is **always** URL-encoded, even when the target is a file
//!   path or a shell command rather than a URL;
//! * only the **first** `{...}` pair is substituted — a template with two
//!   placeholders keeps everything after the first `}` verbatim;
//! * when the template has no braces the parameter is dropped entirely and the
//!   environment-expanded template is returned unchanged.

use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};

/// `Uri.EscapeDataString` leaves the RFC 3986 unreserved set alone:
/// `A-Z a-z 0-9 - . _ ~`.
const UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// `Uri.EscapeDataString(value)`.
pub fn escape_data_string(value: &str) -> String {
    utf8_percent_encode(value, UNRESERVED).to_string()
}

/// `Environment.ExpandEnvironmentVariables(input)`.
///
/// Win32 semantics: `%NAME%` is replaced when `NAME` resolves, and left exactly
/// as written when it does not. A lone `%` is never an error. Implemented in
/// pure Rust rather than through `ExpandEnvironmentStringsW` so the expansion
/// rules are unit-testable on any host.
pub fn expand_environment_variables(input: &str) -> String {
    let bytes: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] != '%' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }

        // Look for the closing '%' of a `%NAME%` pair.
        match bytes[i + 1..].iter().position(|c| *c == '%') {
            Some(offset) => {
                let close = i + 1 + offset;
                let name: String = bytes[i + 1..close].iter().collect();
                // `%%` and `%` + non-resolving name are emitted verbatim.
                match lookup_env(&name) {
                    Some(value) => {
                        out.push_str(&value);
                        i = close + 1;
                    }
                    None => {
                        out.push('%');
                        i += 1;
                    }
                }
            }
            None => {
                out.push('%');
                i += 1;
            }
        }
    }

    out
}

/// Windows environment lookups are case-insensitive; `std::env::var` already is
/// on Windows, but the explicit fallback keeps the behaviour on other hosts.
fn lookup_env(name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    if let Ok(value) = std::env::var(name) {
        return Some(value);
    }
    std::env::vars()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v)
}

/// `MainViewModel.Expand(template, param)`.
pub fn expand(template: &str, param: &str) -> String {
    let encoded = escape_data_string(param);
    let expanded = expand_environment_variables(template);

    let start = expanded.find('{');
    let end = expanded.find('}');

    if let (Some(start), Some(end)) = (start, end) {
        if end > start {
            // `{` and `}` are ASCII, so these byte offsets are always char boundaries.
            return format!("{}{}{}", &expanded[..start], encoded, &expanded[end + 1..]);
        }
    }

    expanded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_the_placeholder_and_url_encodes_the_parameter() {
        assert_eq!(
            expand("https://www.google.com/search?q={Query}", "rust lang"),
            "https://www.google.com/search?q=rust%20lang"
        );
    }

    #[test]
    fn escape_matches_uri_escape_data_string() {
        // Unreserved characters are untouched...
        assert_eq!(escape_data_string("aZ0-._~"), "aZ0-._~");
        // ...everything else, including '+' and '/', is percent-encoded.
        assert_eq!(escape_data_string("a+b/c d&e"), "a%2Bb%2Fc%20d%26e");
        assert_eq!(escape_data_string("café"), "caf%C3%A9");
    }

    #[test]
    fn only_the_first_placeholder_is_replaced() {
        // Faithful to the C#: the text after the first '}' is kept verbatim,
        // so the second placeholder survives into the launched target.
        assert_eq!(expand("a{one}b{two}c", "X"), "aXb{two}c");
    }

    #[test]
    fn a_template_without_braces_ignores_the_parameter() {
        assert_eq!(
            expand("https://github.com/", "ignored"),
            "https://github.com/"
        );
    }

    #[test]
    fn a_reversed_brace_pair_is_left_alone() {
        assert_eq!(expand("}oops{", "X"), "}oops{");
    }

    #[test]
    fn environment_variables_expand_like_win32() {
        std::env::set_var("QGO_TEST_VAR", "hello");
        assert_eq!(expand_environment_variables("%QGO_TEST_VAR%/x"), "hello/x");
        // Unresolvable names survive verbatim, exactly as Win32 leaves them.
        assert_eq!(
            expand_environment_variables("%QGO_NOT_SET_ANYWHERE%"),
            "%QGO_NOT_SET_ANYWHERE%"
        );
        assert_eq!(expand_environment_variables("100% done"), "100% done");
        assert_eq!(expand_environment_variables("no vars here"), "no vars here");
    }

    #[test]
    fn environment_expansion_happens_before_substitution() {
        std::env::set_var("QGO_TEST_ROOT", "C:\\root");
        assert_eq!(expand("%QGO_TEST_ROOT%\\{sub}", "docs"), "C:\\root\\docs");
    }
}
