//! Parsing the explicit plugin management family.

use std::collections::VecDeque;
use std::ffi::OsString;

use super::{Command, ParseError, is_help, required_text};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PluginAction {
    Add { source: String, yes: bool },
    List,
    Update { name: Option<String>, yes: bool },
    Remove { name: String, delete_data: bool },
    Marketplace(MarketplaceAction),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MarketplaceAction {
    Add(String),
    List,
    Update(Option<String>),
    Remove(String),
}

pub(super) fn parse(mut args: VecDeque<OsString>) -> Result<Command, ParseError> {
    args.pop_front();
    if args.iter().any(|argument| is_help(argument.to_str())) {
        return Ok(Command::Help);
    }
    let family = required_text(args.pop_front(), "an action after `plugin`")?;
    let marketplace = family == "marketplace";
    let action = if marketplace {
        required_text(args.pop_front(), "an action after `plugin marketplace`")?
    } else {
        family
    };
    let mut positional = None;
    let mut yes = false;
    let mut delete_data = false;
    for raw in args {
        let argument = required_text(Some(raw), "a UTF-8 plugin argument")?;
        match argument.as_str() {
            "--yes" if !marketplace && matches!(action.as_str(), "add" | "update") && !yes => {
                yes = true
            }
            "--delete-data" if !marketplace && action == "remove" && !delete_data => {
                delete_data = true
            }
            other if other.starts_with('-') => {
                return Err(ParseError::new(format!(
                    "unexpected plugin option `{other}`"
                )));
            }
            _ if positional.is_none() => positional = Some(argument),
            _ => {
                return Err(ParseError::new(format!(
                    "unexpected plugin argument `{argument}`"
                )));
            }
        }
    }
    let required = || {
        positional.clone().ok_or_else(|| {
            ParseError::new(format!(
                "`smith plugin {}{action}` requires {}",
                if marketplace { "marketplace " } else { "" },
                if action == "add" {
                    "a source"
                } else {
                    "a name"
                }
            ))
        })
    };
    let parsed = if marketplace {
        PluginAction::Marketplace(match action.as_str() {
            "add" => MarketplaceAction::Add(required()?),
            "list" if positional.is_none() => MarketplaceAction::List,
            "update" => MarketplaceAction::Update(positional),
            "remove" => MarketplaceAction::Remove(required()?),
            _ => {
                return Err(ParseError::new(
                    "expected `smith plugin marketplace add|list|update|remove`",
                ));
            }
        })
    } else {
        match action.as_str() {
            "add" => PluginAction::Add {
                source: required()?,
                yes,
            },
            "list" if positional.is_none() => PluginAction::List,
            "update" => PluginAction::Update {
                name: positional,
                yes,
            },
            "remove" => PluginAction::Remove {
                name: required()?,
                delete_data,
            },
            _ => {
                return Err(ParseError::new(
                    "expected `smith plugin add|list|update|remove`",
                ));
            }
        }
    };
    Ok(Command::Plugin(parsed))
}

#[cfg(test)]
mod tests {
    use super::{MarketplaceAction, PluginAction};
    use crate::cli::{Command, parse};
    use std::ffi::OsString;

    #[test]
    fn every_plugin_action_is_typed_and_rejects_unrelated_options() {
        for (args, expected) in [
            (
                vec!["plugin", "add", "./bundle", "--yes"],
                PluginAction::Add {
                    source: "./bundle".into(),
                    yes: true,
                },
            ),
            (vec!["plugin", "list"], PluginAction::List),
            (
                vec!["plugin", "update", "--yes"],
                PluginAction::Update {
                    name: None,
                    yes: true,
                },
            ),
            (
                vec!["plugin", "update", "tools"],
                PluginAction::Update {
                    name: Some("tools".into()),
                    yes: false,
                },
            ),
            (
                vec!["plugin", "remove", "tools", "--delete-data"],
                PluginAction::Remove {
                    name: "tools".into(),
                    delete_data: true,
                },
            ),
            (
                vec!["plugin", "marketplace", "add", "./catalog"],
                PluginAction::Marketplace(MarketplaceAction::Add("./catalog".into())),
            ),
            (
                vec!["plugin", "marketplace", "list"],
                PluginAction::Marketplace(MarketplaceAction::List),
            ),
            (
                vec!["plugin", "marketplace", "update"],
                PluginAction::Marketplace(MarketplaceAction::Update(None)),
            ),
            (
                vec!["plugin", "marketplace", "remove", "catalog"],
                PluginAction::Marketplace(MarketplaceAction::Remove("catalog".into())),
            ),
        ] {
            assert_eq!(
                parse(args.into_iter().map(OsString::from)).unwrap(),
                Command::Plugin(expected)
            );
        }
        for args in [
            vec!["plugin"],
            vec!["plugin", "add"],
            vec!["plugin", "list", "--yes"],
            vec!["plugin", "update", "--delete-data"],
            vec!["plugin", "remove"],
            vec!["plugin", "add", "./bundle", "--yes", "--yes"],
            vec!["plugin", "marketplace", "list", "extra"],
        ] {
            assert!(parse(args.into_iter().map(OsString::from)).is_err());
        }
    }
}
