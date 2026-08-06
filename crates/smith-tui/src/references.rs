//! Pure parsing for the composer `@` reference surface.
//!
//! The parser only accepts identities supplied by the host's bounded local
//! index. It performs no filesystem access and cannot turn repository text
//! into an agent preset or authority grant.

use std::collections::BTreeSet;

/// Maximum references accepted from one composer submission.
pub const MAX_COMPOSER_REFERENCES: usize = 16;

/// One locally resolved composer reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerReference {
    /// Canonical workspace-relative file identity.
    File(String),
    /// Host-registered child preset identity.
    Agent(String),
}

/// A locally validated composer submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedReferences {
    /// Prompt text with `@@` escapes reduced to one literal `@`.
    pub text: String,
    /// References in first-occurrence order.
    pub references: Vec<ComposerReference>,
}

/// Resolves token-boundary `@` references against exact local inventories.
pub fn parse_references(
    input: &str,
    files: &BTreeSet<String>,
    agents: &BTreeSet<String>,
) -> Result<ParsedReferences, String> {
    let chars = input.chars().collect::<Vec<_>>();
    let mut text = String::with_capacity(input.len());
    let mut references = Vec::new();
    let mut seen = BTreeSet::new();
    let mut index = 0usize;

    while index < chars.len() {
        if chars[index] != '@' || !is_token_boundary(&chars, index) {
            text.push(chars[index]);
            index += 1;
            continue;
        }
        if chars.get(index + 1) == Some(&'@') {
            text.push('@');
            index += 2;
            continue;
        }

        let mut end = index + 1;
        while end < chars.len() && !chars[end].is_whitespace() {
            end += 1;
        }
        let mut identity_end = end;
        while identity_end > index + 1
            && matches!(chars[identity_end - 1], ',' | ';' | ')' | ']' | '}')
        {
            identity_end -= 1;
        }
        let token = chars[index + 1..identity_end].iter().collect::<String>();
        if token.is_empty() {
            return Err("`@` must name a selectable workspace file or child agent".to_owned());
        }

        let (identity, reference) = if let Some(identity) = token.strip_prefix("file:") {
            let identity = identity.to_owned();
            if !files.contains(&identity) {
                return Err(unresolved(&token));
            }
            (identity.clone(), ComposerReference::File(identity))
        } else if let Some(identity) = token.strip_prefix("agent:") {
            let identity = identity.to_owned();
            if !agents.contains(&identity) {
                return Err(unresolved(&token));
            }
            (identity.clone(), ComposerReference::Agent(identity))
        } else {
            match (files.contains(&token), agents.contains(&token)) {
                (true, false) => (token.clone(), ComposerReference::File(token.clone())),
                (false, true) => (token.clone(), ComposerReference::Agent(token.clone())),
                (true, true) => {
                    return Err(format!(
                        "ambiguous reference `@{}`; use `@file:{}` or `@agent:{}`",
                        bounded_identity(&token),
                        bounded_identity(&token),
                        bounded_identity(&token)
                    ));
                }
                (false, false) => {
                    // An unresolvable bare @token is literal text, not an
                    // error. This lets users type npm scope names
                    // (@forgeailab/smith) or any @ mention that is not a
                    // workspace file or child agent. Explicit typed prefixes
                    // (@file:, @agent:) and ambiguous collisions still error.
                    text.extend(chars[index..end].iter());
                    index = end;
                    continue;
                }
            }
        };
        if seen.insert((reference_kind(&reference), identity.clone())) {
            references.push(reference);
            if references.len() > MAX_COMPOSER_REFERENCES {
                return Err(format!(
                    "a prompt may contain at most {MAX_COMPOSER_REFERENCES} references"
                ));
            }
        }
        text.extend(chars[index..end].iter());
        index = end;
    }

    Ok(ParsedReferences { text, references })
}

fn unresolved(identity: &str) -> String {
    format!(
        "unresolved reference `@{}`; choose it from `@` completion",
        bounded_identity(identity)
    )
}

fn is_token_boundary(chars: &[char], index: usize) -> bool {
    index == 0
        || chars[index - 1].is_whitespace()
        || matches!(chars[index - 1], '(' | '[' | '{' | ',' | ';' | ':')
}

fn reference_kind(reference: &ComposerReference) -> u8 {
    match reference {
        ComposerReference::File(_) => 0,
        ComposerReference::Agent(_) => 1,
    }
}

fn bounded_identity(identity: &str) -> String {
    const MAX_CHARS: usize = 96;
    if identity.chars().count() <= MAX_CHARS {
        return identity.to_owned();
    }
    let mut bounded = identity.chars().take(MAX_CHARS).collect::<String>();
    bounded.push('…');
    bounded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> BTreeSet<String> {
        ["src/lib.rs", "docs/guide.md"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    fn agents() -> BTreeSet<String> {
        ["explore", "review"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn resolves_files_agents_and_literal_escapes() {
        let parsed = parse_references(
            "ask @review about @src/lib.rs and send @@owner",
            &files(),
            &agents(),
        )
        .expect("references");
        assert_eq!(parsed.text, "ask @review about @src/lib.rs and send @owner");
        assert_eq!(
            parsed.references,
            [
                ComposerReference::Agent("review".into()),
                ComposerReference::File("src/lib.rs".into()),
            ]
        );
    }

    #[test]
    fn ignores_email_like_at_signs_outside_token_boundaries() {
        let parsed = parse_references("mail dev@example.com", &files(), &agents()).unwrap();
        assert!(parsed.references.is_empty());
        assert_eq!(parsed.text, "mail dev@example.com");
    }

    #[test]
    fn bare_unresolved_at_token_is_literal_text() {
        let parsed = parse_references("inspect @../secret", &files(), &agents()).unwrap();
        assert!(parsed.references.is_empty());
        assert_eq!(parsed.text, "inspect @../secret");
    }

    #[test]
    fn npm_scoped_package_name_is_literal_text() {
        let parsed = parse_references("run npx @forgeailab/smith", &files(), &agents()).unwrap();
        assert!(parsed.references.is_empty());
        assert_eq!(parsed.text, "run npx @forgeailab/smith");
    }

    #[test]
    fn explicit_typed_unresolved_references_still_fail() {
        let error = parse_references("@file:missing.rs", &files(), &agents()).unwrap_err();
        assert!(
            error.contains("unresolved reference `@file:missing.rs`"),
            "{error}"
        );
        let error = parse_references("@agent:ghost", &files(), &agents()).unwrap_err();
        assert!(
            error.contains("unresolved reference `@agent:ghost`"),
            "{error}"
        );
    }

    #[test]
    fn a_colliding_identity_requires_an_explicit_type() {
        let files = ["review"].into_iter().map(str::to_owned).collect();
        let agents = ["review"].into_iter().map(str::to_owned).collect();
        let error = parse_references("@review inspect", &files, &agents).unwrap_err();
        assert!(error.contains("ambiguous reference"), "{error}");
        assert_eq!(
            parse_references("@file:review", &files, &agents)
                .expect("typed file")
                .references,
            vec![ComposerReference::File("review".to_owned())]
        );
        assert_eq!(
            parse_references("@agent:review inspect", &files, &agents)
                .expect("typed agent")
                .references,
            vec![ComposerReference::Agent("review".to_owned())]
        );
    }
}
