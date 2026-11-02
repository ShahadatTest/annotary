//! Command-line surface: clap definitions, global flags, dispatch, error/exit handling.
//!
//! Every verb returns `CliResult<()>`; `run` prints the error in the mode's format and maps it to
//! the exit code. clap usage errors exit 1 (not clap's default 2); `-h`/`-V` exit 0.

pub mod act;
// ---- ticket 13 ----
pub mod daemon;
// ---- end ticket 13 ----
pub mod devices; // CMD-23
pub mod docref;
// ---- ticket 13 ----
pub mod export;
// ---- end ticket 13 ----
// ---- ticket 14 ----
pub mod help;
// ---- end ticket 14 ----
pub mod lifecycle;
// ---- ticket 18 ----
pub mod link;
// ---- end ticket 18 ----
pub mod list;
pub mod login;
pub mod member;
pub mod mentions;
// ---- ticket 18 ----
pub mod pair;
pub mod password; // CMD-22
// ---- end ticket 18 ----
pub mod profile;
pub mod prompt;
pub mod pull;
pub mod push;
// ---- ticket 09 ----
pub mod rerender;
// ---- end ticket 09 ----
pub mod serve;
pub mod settings; // CMD-23
pub mod show;
// ---- ticket 14 ----
pub mod skill;
// ---- end ticket 14 ----
pub mod thread;
pub mod token;
// ---- CMD-5 ----
pub mod vault;
// ---- end CMD-5 ----
pub mod versions;
// ---- ticket 08 ----
pub mod wait;
// ---- end ticket 08 ----

use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Args, CommandFactory, Parser, Subcommand};

use crate::client::{Api, Credential, EnvVars, resolve_credential};
use crate::exit::{CliResult, Exit};
use crate::output::Out;
use crate::paths::{self, DataDir};

/// annotary — share Markdown with humans and agents; comment on it; act on it.
#[derive(Parser, Debug)]
#[command(
    name = "annotary",
    version = crate::VERSION,
    about = "share Markdown with humans and agents; comment on it; act on it.",
    // ticket 14: the prototype's hand-written help (grouped verbs, the global options once, DOC
    // + exit codes) — see `help.rs`.
    help_template = help::TOP,
    disable_help_subcommand = true,
    subcommand_required = true,
    arg_required_else_help = true,
    max_term_width = 100
)]
pub struct Cli {
    #[command(flatten)]
    pub globals: Globals,
    #[command(subcommand)]
    pub cmd: Command,
}

/// Flags every verb accepts.
#[derive(Args, Debug, Clone, Default)]
pub struct Globals {
    /// Profile from ~/.annotary/config.toml (who you are; beats ANNOTARY_TOKEN / ANNOTARY_PROFILE)
    #[arg(short = 'p', long, global = true, value_name = "NAME")]
    pub profile: Option<String>,
    /// JSON output; FIELDS narrows (comma list), '?' lists fields. Put it after the verb.
    #[arg(long, global = true, value_name = "FIELDS", num_args = 0..=1, default_missing_value = "")]
    pub json: Option<String>,
    /// Result only — no progress, no hints
    #[arg(short = 'q', long, global = true)]
    pub quiet: bool,
    /// No colour, even on a terminal
    #[arg(long, global = true)]
    pub no_color: bool,
    /// Data dir (config.toml, server.json, logs/, store/) [default: ~/.annotary]
    #[arg(long, global = true, value_name = "DIR", env = "ANNOTARY_HOME")]
    pub data_dir: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Documents in the Vault (id name vN action open updated), most active first
    List(list::ListArgs),
    /// Upload a new Version (or create the Document); re-anchors Threads at the edge
    #[command(help_template = help::PUSH)]
    Push(push::PushArgs),
    /// Write <name>.md + <name>.annotary into the current dir
    #[command(help_template = help::PULL)]
    Pull(pull::PullArgs),
    /// One-line header + open Threads (what an agent reads first)
    Show(show::ShowArgs),
    /// version · action · open Threads · mentions of you · event cursor
    #[command(help_template = help::STATUS)]
    Status(show::ShowArgs),
    /// Version list: number by at message same_as
    Versions(versions::VersionsArgs),
    /// Delete a Document (soft — Versions/Threads/Actions kept; `restore` undoes it)
    Delete(lifecycle::DocArg),
    /// Bring a deleted Document back
    Restore(lifecycle::DocArg),
    /// Permanently remove a deleted Document and reclaim its blobs (owner)
    Purge(lifecycle::DocArg),
    // ---- ticket 18 ----
    /// The URL that opens a Document (or one of its Threads) on another device
    #[command(help_template = help::LINK)]
    Link(link::LinkArgs),
    // ---- end ticket 18 ----
    /// Threads: list | show | new | reply | resolve | reopen | edit | delete
    #[command(help_template = help::THREAD)]
    Thread {
        #[command(subcommand)]
        cmd: thread::ThreadCmd,
    },
    /// Open Threads mentioning you across every Document you can read (--since)
    Mentions(mentions::MentionsArgs),
    // ---- ticket 08 ----
    /// Block until a new Version, Action or @mention (then print it and exit); --cursor resumes
    #[command(help_template = help::WAIT)]
    Wait(wait::WaitArgs),
    // ---- end ticket 08 ----
    // ---- ticket 09 ----
    /// Re-render a Document's HTML (every Version) with the server's renderer; --all for every Document (owner)
    Rerender(rerender::RerenderArgs),
    // ---- end ticket 09 ----
    /// Run the local server in the foreground (127.0.0.1:7377); first run sets up the Tenant
    Serve(serve::ServeArgs),
    // ---- ticket 13 ----
    /// macOS LaunchAgent around `serve`: install | uninstall | status | start | stop | restart
    #[command(help_template = help::DAEMON)]
    Daemon {
        #[command(subcommand)]
        cmd: daemon::DaemonCmd,
    },
    /// Archive the store: a consistent tar of store/ (works beside a running server)
    #[command(after_help = export::AFTER_HELP)]
    Export(export::ExportArgs),
    // ---- end ticket 13 ----
    // ---- CMD-22 ----
    /// Clear or set a local password in a stopped data dir's store (no server, no credential)
    #[command(after_help = password::AFTER_HELP)]
    Password(password::PasswordArgs),
    // ---- end CMD-22 ----
    /// Members: add | list | rename | disable | grant | role  (owner)
    #[command(help_template = help::MEMBER)]
    Member {
        #[command(subcommand)]
        cmd: member::MemberCmd,
    },
    // ---- ticket 05 ----
    /// Take an Action on head (--version N guards against a moved head)
    #[command(help_template = help::ACT)]
    Act(act::ActArgs),
    /// Action history, newest first (id by at version label)
    Actions(act::ActionsArgs),
    // ---- end ticket 05 ----
    // ---- ticket 06 ----
    /// Tokens: mint | list | revoke  (yours; owner: anyone's)
    Token {
        #[command(subcommand)]
        cmd: token::TokenCmd,
    },
    // ---- CMD-5 ----
    /// Vaults: list | create | rename | delete  (owner; list: anyone)
    Vault {
        #[command(subcommand)]
        cmd: vault::VaultCmd,
    },
    // ---- end CMD-5 ----
    // ---- CMD-23 ----
    /// Your settings: list | get | set | reset  (device settings: --all-devices | --device NAME)
    Settings {
        #[command(subcommand)]
        cmd: Option<settings::SettingsCmd>,
    },
    /// Your Devices: list | rename | forget
    Devices {
        #[command(subcommand)]
        cmd: Option<devices::DevicesCmd>,
    },
    // ---- end CMD-23 ----
    // ---- ticket 18 ----
    /// Pair a phone or tablet: a new Token as a annotary://pair QR code
    #[command(help_template = help::PAIR)]
    Pair(pair::PairArgs),
    // ---- end ticket 18 ----
    /// Get a token with the local password and save it as a profile
    Login(login::LoginArgs),
    /// Revoke the profile's token and forget it
    Logout(login::LogoutArgs),
    /// Profiles in ~/.annotary/config.toml: list | add | remove
    Profile {
        #[command(subcommand)]
        cmd: profile::ProfileCmd,
    },
    // ---- end ticket 06 ----
    // ---- ticket 14 ----
    /// Agent skill file: `skill install [--dir DIR]` writes this CLI version's SKILL.md
    #[command(help_template = help::SKILL)]
    Skill {
        #[command(subcommand)]
        cmd: skill::SkillCmd,
    },
    // ---- end ticket 14 ----
}

/// Everything a verb needs: resolved output settings, data dir, globals.
pub struct Ctx {
    pub out: Out,
    pub data: DataDir,
    pub globals: Globals,
}

impl Ctx {
    pub fn new(globals: Globals) -> Ctx {
        let out = Out::resolve(globals.json.as_deref(), globals.quiet, globals.no_color);
        let root = paths::data_dir(globals.data_dir.as_deref(), None);
        Ctx { out, data: DataDir::new(root), globals }
    }

    /// Resolve the credential (exit 3 on failure).
    pub fn credential(&self) -> CliResult<Credential> {
        resolve_credential(&self.data.config_toml(), self.globals.profile.as_deref(), &EnvVars::from_process())
    }

    /// The profile named by `-p` or, without one, `ANNOTARY_PROFILE` (`login`/`logout`).
    pub fn profile_name(&self) -> Option<String> {
        self.globals
            .profile
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .or(EnvVars::from_process().profile)
    }

    /// An API client for the resolved credential. CMD-5: it carries the profile's Vault and
    /// heals that profile in `config.toml` when the server says the Vault was renamed or deleted.
    pub fn api(&self) -> CliResult<Api> {
        let cred = self.credential()?;
        Ok(Api::new(&cred).with_profile_vault(&cred, &self.data.config_toml(), self.out.quiet))
    }

    /// The selected profile's `vault`, if any (the Vault scope for `list` and Name resolution,
    /// and the default Vault of a new Document).
    pub fn profile_vault(&self) -> Option<String> {
        self.credential().ok().and_then(|c| c.vault).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
    }
}

/// Parse and run; returns the process exit code.
pub fn run<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(e) => {
            use clap::error::ErrorKind;
            match e.kind() {
                ErrorKind::DisplayVersion => {
                    let _ = e.print();
                    crate::update::after_version();
                    return Exit::Ok.code();
                }
                ErrorKind::DisplayHelp => {
                    let _ = e.print();
                    return Exit::Ok.code();
                }
                _ => {
                    let _ = e.print();
                    return Exit::Usage.code();
                }
            }
        }
    };
    let ctx = Ctx::new(cli.globals);
    let result = match cli.cmd {
        Command::List(args) => list::run(&ctx, args),
        Command::Push(args) => push::run(&ctx, args),
        Command::Pull(args) => pull::run(&ctx, args),
        Command::Show(args) => show::show(&ctx, args),
        Command::Status(args) => show::status(&ctx, args),
        Command::Versions(args) => versions::run(&ctx, args),
        Command::Delete(args) => lifecycle::run(&ctx, lifecycle::Op::Delete, args),
        Command::Restore(args) => lifecycle::run(&ctx, lifecycle::Op::Restore, args),
        Command::Purge(args) => lifecycle::run(&ctx, lifecycle::Op::Purge, args),
        Command::Link(args) => link::run(&ctx, args), // ticket 18
        Command::Thread { cmd } => thread::run(&ctx, cmd),
        Command::Mentions(args) => mentions::run(&ctx, args),
        Command::Wait(args) => wait::run(&ctx, args),         // ticket 08
        Command::Rerender(args) => rerender::run(&ctx, args), // ticket 09
        Command::Serve(args) => serve::run(&ctx, args),
        // ---- ticket 13 ----
        Command::Daemon { cmd } => daemon::run(&ctx, cmd),
        Command::Export(args) => export::run(&ctx, args),
        Command::Password(args) => password::run(&ctx, args),
        // ---- end ticket 13 ----
        Command::Member { cmd } => member::run(&ctx, cmd),
        // ---- ticket 05 ----
        Command::Act(args) => act::act(&ctx, args),
        Command::Actions(args) => act::actions(&ctx, args),
        // ---- end ticket 05 ----
        // ---- ticket 06 ----
        Command::Token { cmd } => token::run(&ctx, cmd),
        Command::Vault { cmd } => vault::run(&ctx, cmd),       // CMD-5
        Command::Settings { cmd } => settings::run(&ctx, cmd), // CMD-23
        Command::Devices { cmd } => devices::run(&ctx, cmd),   // CMD-23
        Command::Pair(args) => pair::run(&ctx, args),          // ticket 18
        Command::Login(args) => login::login(&ctx, args),
        Command::Logout(args) => login::logout(&ctx, args),
        Command::Profile { cmd } => profile::run(&ctx, cmd),
        // ---- end ticket 06 ----
        // ---- ticket 14 ----
        Command::Skill { cmd } => skill::run(&ctx, cmd),
        // ---- end ticket 14 ----
    };
    match result {
        Ok(()) => Exit::Ok.code(),
        Err(err) => {
            ctx.out.error(&err);
            err.exit.code()
        }
    }
}

/// The clap command (for help-text tests).
pub fn command() -> clap::Command {
    Cli::command()
}
