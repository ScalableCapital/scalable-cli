use std::ffi::OsString;
use std::iter::Peekable;

use clap::error::{ContextKind, ContextValue, Error as ClapError};
use clap::{Arg, Command, CommandFactory};

use crate::cli::Cli;
use crate::cli::Commands;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OutputMode {
    Human,
    Machine,
}

impl OutputMode {
    pub(crate) fn from_json_requested(json_requested: bool) -> Self {
        if json_requested {
            Self::Machine
        } else {
            Self::Human
        }
    }

    pub(crate) fn requests_machine_output(self) -> bool {
        matches!(self, Self::Machine)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExecutionContext {
    pub(crate) output_mode: OutputMode,
    pub(crate) command_name: &'static str,
}

impl ExecutionContext {
    pub(crate) fn from_command(command: &Commands) -> Self {
        Self {
            output_mode: OutputMode::from_json_requested(crate::command_requests_json_envelope(
                command,
            )),
            command_name: crate::machine_command_name(command),
        }
    }

    pub(crate) fn from_raw_args_and_command(raw_args: &[OsString], command: &Commands) -> Self {
        Self {
            output_mode: OutputMode::from_json_requested(raw_args_request_json(raw_args)),
            command_name: crate::machine_command_name(command),
        }
    }

    pub(crate) fn requests_machine_output(self) -> bool {
        self.output_mode.requests_machine_output()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParseFailureContext {
    pub(crate) output_mode: OutputMode,
    pub(crate) command_name: String,
}

impl ParseFailureContext {
    pub(crate) fn from_raw_args_and_error(
        raw_args: &[OsString],
        error: &ClapError,
    ) -> ParseFailureContext {
        parse_failure_context_for_command(&Cli::command(), raw_args, error)
    }

    pub(crate) fn requests_machine_output(&self) -> bool {
        self.output_mode.requests_machine_output()
    }
}

fn raw_args_request_json(raw_args: &[OsString]) -> bool {
    raw_args
        .iter()
        .take_while(|arg| arg.as_os_str() != "--")
        .any(|arg| arg == "--json")
}

fn parse_failure_context_for_command(
    command: &Command,
    raw_args: &[OsString],
    error: &ClapError,
) -> ParseFailureContext {
    let schema_match = raw_args_command_match_for_command(command, raw_args);
    let command_name = parse_failure_command_name(error, &schema_match);
    ParseFailureContext {
        output_mode: OutputMode::from_json_requested(
            raw_args_request_json(raw_args) && schema_match.supports_machine_output,
        ),
        command_name,
    }
}

fn parse_failure_command_name(error: &ClapError, schema_match: &RawArgsCommandMatch) -> String {
    usage_command_name(error)
        .or_else(|| prior_arg_command_name(error))
        .or_else(|| schema_match.command_name.clone())
        .unwrap_or_else(|| "sc".to_string())
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct RawArgsCommandMatch {
    command_name: Option<String>,
    supports_machine_output: bool,
}

#[cfg(test)]
fn raw_args_command_match(raw_args: &[OsString]) -> RawArgsCommandMatch {
    raw_args_command_match_for_command(&Cli::command(), raw_args)
}

fn raw_args_command_match_for_command(
    command: &Command,
    raw_args: &[OsString],
) -> RawArgsCommandMatch {
    let mut current = command;
    let mut command_path = Vec::new();
    let mut tokens = raw_args.iter().skip(1).peekable();

    while let Some(raw_token) = tokens.next() {
        if raw_token == "--" {
            break;
        }

        let Some(token) = raw_token.to_str() else {
            break;
        };

        if token.starts_with("--") {
            if !consume_long_option(token, current, &mut tokens) {
                break;
            }
            continue;
        }

        if is_short_option(token) {
            if !consume_short_option(token, current, &mut tokens) {
                break;
            }
            continue;
        }

        let Some(subcommand) = current.find_subcommand(token) else {
            break;
        };
        command_path.push(subcommand.get_name().to_string());
        current = subcommand;
    }

    RawArgsCommandMatch {
        supports_machine_output: !command_path.is_empty() && command_tree_supports_json(current),
        command_name: (!command_path.is_empty()).then(|| command_path.join(".")),
    }
}

fn consume_long_option<'a, I>(token: &str, command: &Command, tokens: &mut Peekable<I>) -> bool
where
    I: Iterator<Item = &'a OsString>,
{
    let option_name = token
        .trim_start_matches("--")
        .split_once('=')
        .map(|(name, _)| name)
        .unwrap_or_else(|| token.trim_start_matches("--"));
    let Some(arg) = find_long_option(command, option_name) else {
        return false;
    };

    if arg_takes_values(arg) && !token.contains('=') {
        consume_option_values(arg, tokens);
    }

    true
}

fn consume_short_option<'a, I>(token: &str, command: &Command, tokens: &mut Peekable<I>) -> bool
where
    I: Iterator<Item = &'a OsString>,
{
    let Some(shorts) = token.strip_prefix('-') else {
        return false;
    };
    if shorts.is_empty() {
        return false;
    }

    for (offset, short) in shorts.char_indices() {
        let Some(arg) = find_short_option(command, short) else {
            return false;
        };
        if arg_takes_values(arg) {
            let attached_value = &shorts[offset + short.len_utf8()..];
            if attached_value.is_empty() {
                consume_option_values(arg, tokens);
            }
            break;
        }
    }

    true
}

fn consume_option_values<'a, I>(arg: &Arg, tokens: &mut Peekable<I>)
where
    I: Iterator<Item = &'a OsString>,
{
    let value_count = arg
        .get_num_args()
        .map(|num_args| num_args.min_values().max(1))
        .unwrap_or(1);

    for _ in 0..value_count {
        let Some(next) = tokens.peek() else {
            break;
        };
        let Some(next_token) = next.to_str() else {
            break;
        };
        if next_token == "--" || next_token.starts_with('-') {
            break;
        }
        tokens.next();
    }
}

fn find_long_option<'a>(command: &'a Command, option_name: &str) -> Option<&'a Arg> {
    command
        .get_arguments()
        .find(|arg| long_option_names(arg).any(|name| name == option_name))
}

fn find_short_option(command: &Command, option_name: char) -> Option<&Arg> {
    command
        .get_arguments()
        .find(|arg| short_option_names(arg).any(|name| name == option_name))
}

fn long_option_names(arg: &Arg) -> impl Iterator<Item = &str> {
    arg.get_long()
        .into_iter()
        .chain(arg.get_all_aliases().into_iter().flatten())
}

fn short_option_names(arg: &Arg) -> impl Iterator<Item = char> {
    arg.get_short()
        .into_iter()
        .chain(arg.get_all_short_aliases().into_iter().flatten())
}

fn arg_takes_values(arg: &Arg) -> bool {
    arg.get_action().takes_values()
}

fn command_tree_supports_json(command: &Command) -> bool {
    command_supports_json(command) || command.get_subcommands().any(command_tree_supports_json)
}

fn command_supports_json(command: &Command) -> bool {
    command
        .get_arguments()
        .any(|arg| arg.get_long() == Some("json"))
}

fn is_short_option(token: &str) -> bool {
    token.starts_with('-') && !token.starts_with("--")
}

fn usage_command_name(error: &ClapError) -> Option<String> {
    let usage = match error.get(ContextKind::Usage) {
        Some(ContextValue::StyledStr(usage)) => usage.to_string(),
        Some(other) => other.to_string(),
        None => return None,
    };

    let tokens = usage
        .lines()
        .find_map(|line| line.strip_prefix("Usage: "))
        .or_else(|| usage.lines().next())
        .map(str::trim)?;

    let mut command_parts = Vec::new();
    for token in tokens
        .split_whitespace()
        .skip_while(|token| *token != "sc")
        .skip(1)
    {
        if token.starts_with('[') || token.starts_with('<') || token.starts_with('-') {
            break;
        }
        command_parts.push(token);
    }

    if command_parts.is_empty() {
        Some("sc".to_string())
    } else {
        Some(command_parts.join("."))
    }
}

fn prior_arg_command_name(error: &ClapError) -> Option<String> {
    match error.get(ContextKind::PriorArg) {
        Some(ContextValue::String(value)) => normalize_prior_arg(value),
        Some(ContextValue::Strings(values)) => {
            values.iter().find_map(|value| normalize_prior_arg(value))
        }
        Some(other) => normalize_prior_arg(&other.to_string()),
        None => None,
    }
}

fn normalize_prior_arg(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.starts_with('-') {
        return None;
    }

    Some(trimmed.replace(' ', "."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{
        ArgAction, CommandFactory,
        error::{ContextValue, ErrorKind},
    };

    use crate::cli::Cli;

    fn parse_error(args: &[&str]) -> ClapError {
        Cli::command()
            .try_get_matches_from(args)
            .expect_err("expected clap parse error")
    }

    fn synthetic_short_option_command() -> clap::Command {
        clap::Command::new("sc").subcommand(
            clap::Command::new("parent")
                .arg(clap::Arg::new("all").short('a').action(ArgAction::SetTrue))
                .arg(
                    clap::Arg::new("portfolio-id")
                        .short('b')
                        .long("portfolio-id")
                        .num_args(1),
                )
                .subcommand(
                    clap::Command::new("child")
                        .arg(
                            clap::Arg::new("json")
                                .long("json")
                                .action(ArgAction::SetTrue),
                        )
                        .arg(
                            clap::Arg::new("required-id")
                                .long("required-id")
                                .required(true),
                        ),
                ),
        )
    }

    #[test]
    fn raw_args_detect_machine_mode() {
        let args = vec![
            OsString::from("sc"),
            OsString::from("capabilities"),
            OsString::from("--json"),
        ];
        let error = parse_error(&["sc", "broker", "bogus"]);
        let context = ParseFailureContext::from_raw_args_and_error(&args, &error);
        assert!(context.requests_machine_output());
    }

    #[test]
    fn raw_args_ignore_json_after_double_dash() {
        let args = vec![
            OsString::from("sc"),
            OsString::from("capabilities"),
            OsString::from("--"),
            OsString::from("--json"),
        ];
        let error = parse_error(&["sc", "broker", "bogus"]);
        let context = ParseFailureContext::from_raw_args_and_error(&args, &error);
        assert!(!context.requests_machine_output());
    }

    #[test]
    fn parse_failure_uses_nested_usage_command_name() {
        let error = parse_error(&["sc", "broker", "context", "select", "--json"]);
        assert_eq!(
            parse_failure_command_name(&error, &RawArgsCommandMatch::default()),
            "broker.context.select"
        );
    }

    #[test]
    fn invalid_subcommand_falls_back_to_parent_usage_path() {
        let error = parse_error(&["sc", "broker", "bogus", "--json"]);
        assert_eq!(
            parse_failure_command_name(&error, &RawArgsCommandMatch::default()),
            "broker"
        );
    }

    #[test]
    fn usage_parser_stops_before_options() {
        let error = parse_error(&["sc", "broker", "chart", "--json"]);
        assert_eq!(
            parse_failure_command_name(&error, &RawArgsCommandMatch::default()),
            "broker.chart"
        );
    }

    #[test]
    fn pre_normalization_context_uses_raw_json_flag() {
        let command = Commands::Broker(crate::cli::BrokerArgs {
            command: crate::cli::BrokerCommand::Watchlist(crate::cli::BrokerWatchlistArgs {
                command: Some(crate::cli::BrokerWatchlistCommand::Add(
                    crate::cli::BrokerWatchlistAddArgs {
                        portfolio_id: None,
                        isin: "US0378331005".to_string(),
                        json: false,
                    },
                )),
                portfolio_id: None,
                include_year_to_date: false,
                json: false,
            }),
        });
        let raw_args = vec![
            OsString::from("sc"),
            OsString::from("broker"),
            OsString::from("watchlist"),
            OsString::from("--json"),
            OsString::from("add"),
            OsString::from("--isin"),
            OsString::from("US0378331005"),
        ];

        let context = ExecutionContext::from_raw_args_and_command(&raw_args, &command);
        assert!(context.requests_machine_output());
        assert_eq!(context.command_name, "broker.watchlist.add");
    }

    #[test]
    fn parse_failure_uses_prior_arg_when_usage_is_missing() {
        let mut error = ClapError::raw(ErrorKind::InvalidSubcommand, "invalid subcommand");
        error.insert(
            ContextKind::PriorArg,
            ContextValue::String("broker trade".to_string()),
        );

        assert_eq!(
            parse_failure_command_name(&error, &RawArgsCommandMatch::default()),
            "broker.trade"
        );
    }

    #[test]
    fn parse_failure_falls_back_to_root_when_context_is_missing() {
        let error = ClapError::raw(ErrorKind::UnknownArgument, "unexpected argument");
        assert_eq!(
            parse_failure_command_name(&error, &RawArgsCommandMatch::default()),
            "sc"
        );
    }

    #[test]
    fn parse_failure_falls_back_to_raw_args_for_invalid_value_leaf_command() {
        let raw_args = vec![
            OsString::from("sc"),
            OsString::from("broker"),
            OsString::from("chart"),
            OsString::from("--isin"),
            OsString::from("US0378331005"),
            OsString::from("--timeframe"),
            OsString::from("nope"),
            OsString::from("--json"),
        ];
        let error = ClapError::raw(ErrorKind::InvalidValue, "invalid value");

        assert_eq!(
            parse_failure_command_name(&error, &raw_args_command_match(&raw_args)),
            "broker.chart"
        );
    }

    #[test]
    fn parse_failure_json_mode_is_disabled_for_human_only_commands() {
        let raw_args = vec![
            OsString::from("sc"),
            OsString::from("login"),
            OsString::from("--json"),
        ];
        let error = ClapError::raw(ErrorKind::UnknownArgument, "unexpected argument");

        let context = ParseFailureContext::from_raw_args_and_error(&raw_args, &error);
        assert_eq!(context.command_name, "login");
        assert!(!context.requests_machine_output());
    }

    #[test]
    fn raw_args_command_name_skips_parent_flag_values_before_nested_subcommand() {
        let raw_args = vec![
            OsString::from("sc"),
            OsString::from("broker"),
            OsString::from("watchlist"),
            OsString::from("--portfolio-id"),
            OsString::from("portfolio-1"),
            OsString::from("--json"),
            OsString::from("add"),
            OsString::from("--isin"),
            OsString::from("US0378331005"),
        ];

        assert_eq!(
            raw_args_command_match(&raw_args).command_name.as_deref(),
            Some("broker.watchlist.add")
        );
    }

    #[test]
    fn raw_args_machine_output_support_comes_from_command_subtree() {
        let raw_args = vec![
            OsString::from("sc"),
            OsString::from("broker"),
            OsString::from("transaction"),
            OsString::from("--json"),
        ];

        let context = raw_args_command_match(&raw_args);
        assert_eq!(context.command_name.as_deref(), Some("broker.transaction"));
        assert!(context.supports_machine_output);
    }

    #[test]
    fn root_command_does_not_enable_machine_output_from_descendants() {
        let raw_args = vec![OsString::from("sc"), OsString::from("--json")];

        let context = raw_args_command_match(&raw_args);
        assert_eq!(context.command_name, None);
        assert!(!context.supports_machine_output);
    }

    #[test]
    fn raw_args_command_match_handles_short_option_with_attached_value() {
        let command = synthetic_short_option_command();
        let raw_args = vec![
            OsString::from("sc"),
            OsString::from("parent"),
            OsString::from("-abportfolio-1"),
            OsString::from("child"),
            OsString::from("--json"),
        ];

        let context = raw_args_command_match_for_command(&command, &raw_args);
        assert_eq!(context.command_name.as_deref(), Some("parent.child"));
        assert!(context.supports_machine_output);
    }

    #[test]
    fn raw_args_command_match_handles_short_option_with_equals_attached_value() {
        let command = synthetic_short_option_command();
        let raw_args = vec![
            OsString::from("sc"),
            OsString::from("parent"),
            OsString::from("-b=portfolio-1"),
            OsString::from("child"),
            OsString::from("--json"),
        ];

        let context = raw_args_command_match_for_command(&command, &raw_args);
        assert_eq!(context.command_name.as_deref(), Some("parent.child"));
        assert!(context.supports_machine_output);
    }

    #[test]
    fn parse_failure_context_handles_clustered_short_option_with_attached_value() {
        let command = synthetic_short_option_command();
        let raw_args = vec![
            OsString::from("sc"),
            OsString::from("parent"),
            OsString::from("-abportfolio-1"),
            OsString::from("child"),
            OsString::from("--json"),
        ];
        let error = command
            .clone()
            .try_get_matches_from(raw_args.clone())
            .expect_err("expected clap parse error");

        let context = parse_failure_context_for_command(&command, &raw_args, &error);
        assert_eq!(context.command_name, "parent.child");
        assert!(context.requests_machine_output());
    }
}
