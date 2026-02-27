use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::cli::RunArgs;
use crate::error::PealError;

// Precedence: CLI > env > file > defaults.

const DEFAULT_AGENT_CMD: &str = "agent";
const DEFAULT_SANDBOX: &str = "disabled";
const DEFAULT_MAX_ADDRESS_ROUNDS: u32 = 5;
const DEFAULT_ON_FINDINGS_REMAINING: &str = "fail";
const DEFAULT_STATE_DIR: &str = ".peal";
const DEFAULT_PHASE_TIMEOUT_SEC: u64 = 1800;
const DEFAULT_PHASE_RETRY_COUNT: u32 = 0;
const DEFAULT_PHASE_3_RETRY_COUNT: u32 = 0;
const DEFAULT_NORMALIZE_RETRY_COUNT: u32 = 0;
const DEFAULT_MAX_PARALLEL: u32 = 4;
const DEFAULT_ON_STET_FAIL: &str = "fail";

/// Valid dismiss reasons for stet (must match `stet dismiss <id> <reason>`).
pub const STET_DISMISS_REASONS: [&str; 4] = [
    "false_positive",
    "already_correct",
    "wrong_suggestion",
    "out_of_scope",
];

/// One pattern to match finding message/path; when matched, dismiss with the given reason.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StetDismissPattern {
    pub pattern: String,
    pub reason: String,
}

impl StetDismissPattern {
    /// Returns true if `reason` is one of the four allowed values.
    pub fn is_valid_reason(reason: &str) -> bool {
        STET_DISMISS_REASONS.contains(&reason)
    }
}

const ENV_PREFIX: &str = "PEAL_";

/// Resolved configuration for a PEAL run.
///
/// Built from three layers with precedence CLI > env > file > defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PealConfig {
    pub agent_cmd: String,
    pub plan_path: PathBuf,
    pub repo_path: PathBuf,
    pub stet_commands: Vec<String>,
    pub sandbox: String,
    pub model: Option<String>,
    pub max_address_rounds: u32,
    pub on_findings_remaining: String,
    pub state_dir: PathBuf,
    pub phase_timeout_sec: u64,
    pub phase_retry_count: u32,
    /// Number of retries for Phase 3 (address findings) Cursor CLI on timeout or non-zero exit. Effective cap 2 at use site.
    pub phase_3_retry_count: u32,
    pub parallel: bool,
    pub max_parallel: u32,
    /// When true, a task failure in a parallel block does not stop the run:
    /// completed indices are persisted, position advances, and the runner returns Ok (exit 0).
    pub continue_with_remaining_tasks: bool,
    pub log_level: Option<String>,
    pub log_file: Option<PathBuf>,
    pub stet_path: Option<PathBuf>,
    pub stet_start_ref: Option<String>,
    /// Extra arguments passed through to `stet start` (e.g. `--allow-dirty`).
    pub stet_start_extra_args: Vec<String>,
    /// Extra arguments passed through to `stet run` (e.g. `--verify`, `--context 256k`).
    /// Peal always adds `--output=json` for run; these are appended so user flags are supported.
    pub stet_run_extra_args: Vec<String>,
    /// When true, do not call the agent to triage findings; use rule-based stet_dismiss_patterns only.
    /// Default false: use LLM with "Anything to address from this review?" to decide what to dismiss.
    pub stet_disable_llm_triage: bool,
    /// When LLM triage is disabled, match finding message/path against these patterns; if match, dismiss with reason.
    /// Empty when LLM triage is enabled or not configured.
    pub stet_dismiss_patterns: Vec<StetDismissPattern>,
    /// Behavior when stet start or stet run fails: "fail" (default), "retry_once", or "skip".
    pub on_stet_fail: String,
    /// Commands run after all tasks succeed (and after stet finish when stet is used).
    /// Working directory = repo_path; stdout/stderr captured and logged; best-effort, no Cursor call.
    /// Each string is split on whitespace: first token = program, rest = args (exec-style, no shell).
    pub post_run_commands: Vec<String>,
    /// Timeout in seconds for each post-run command. When None, phase_timeout_sec is used.
    pub post_run_timeout_sec: Option<u64>,
    /// When true, non-canonical plan files are normalized via Cursor CLI before parsing (SP-7.2).
    pub normalize_plan: bool,
    /// Number of retries for normalize+parse when normalized output fails to parse (SP-7.3). Default 0.
    pub normalize_retry_count: u32,
    /// Optional path to a file whose content is the full normalization prompt; placeholder `{{DOC}}` is replaced by the plan document. If None, built-in prompt is used.
    pub normalize_prompt_path: Option<PathBuf>,
    /// When true, validate Phase 1 plan text after capture (non-empty and optionally minimum length). Off by default.
    pub validate_plan_text: bool,
    /// When validate_plan_text is true: require plan_text.len() >= this value. If None, only non-empty is required.
    pub min_plan_text_len: Option<usize>,
    /// Optional path for run summary JSON. When None, summary is written to state_dir/run_summary.json.
    pub run_summary_path: Option<PathBuf>,
    /// When set, run stops after this many consecutive task failures; state is persisted and exit code 3.
    /// None = cap disabled.
    pub max_consecutive_task_failures: Option<u32>,
}

/// TOML-deserializable config file representation. All fields optional.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    agent_cmd: Option<String>,
    plan_path: Option<PathBuf>,
    repo_path: Option<PathBuf>,
    stet_commands: Option<Vec<String>>,
    sandbox: Option<String>,
    model: Option<String>,
    max_address_rounds: Option<u32>,
    on_findings_remaining: Option<String>,
    state_dir: Option<PathBuf>,
    phase_timeout_sec: Option<u64>,
    phase_retry_count: Option<u32>,
    phase_3_retry_count: Option<u32>,
    parallel: Option<bool>,
    max_parallel: Option<u32>,
    continue_with_remaining_tasks: Option<bool>,
    log_level: Option<String>,
    log_file: Option<PathBuf>,
    stet_path: Option<PathBuf>,
    stet_start_ref: Option<String>,
    stet_start_extra_args: Option<Vec<String>>,
    stet_run_extra_args: Option<Vec<String>>,
    stet_disable_llm_triage: Option<bool>,
    stet_dismiss_patterns: Option<Vec<StetDismissPattern>>,
    on_stet_fail: Option<String>,
    post_run_commands: Option<Vec<String>>,
    post_run_timeout_sec: Option<u64>,
    normalize_plan: Option<bool>,
    normalize_retry_count: Option<u32>,
    normalize_prompt_path: Option<PathBuf>,
    validate_plan_text: Option<bool>,
    min_plan_text_len: Option<u64>,
    run_summary_path: Option<PathBuf>,
    max_consecutive_task_failures: Option<u32>,
}

/// Intermediate layer where every field is optional, used to merge sources.
#[derive(Debug, Default)]
struct ConfigLayer {
    agent_cmd: Option<String>,
    plan_path: Option<PathBuf>,
    repo_path: Option<PathBuf>,
    stet_commands: Option<Vec<String>>,
    sandbox: Option<String>,
    model: Option<String>,
    max_address_rounds: Option<u32>,
    on_findings_remaining: Option<String>,
    state_dir: Option<PathBuf>,
    phase_timeout_sec: Option<u64>,
    phase_retry_count: Option<u32>,
    phase_3_retry_count: Option<u32>,
    parallel: Option<bool>,
    max_parallel: Option<u32>,
    continue_with_remaining_tasks: Option<bool>,
    log_level: Option<String>,
    log_file: Option<PathBuf>,
    stet_path: Option<PathBuf>,
    stet_start_ref: Option<String>,
    stet_start_extra_args: Option<Vec<String>>,
    stet_run_extra_args: Option<Vec<String>>,
    stet_disable_llm_triage: Option<bool>,
    stet_dismiss_patterns: Option<Vec<StetDismissPattern>>,
    on_stet_fail: Option<String>,
    post_run_commands: Option<Vec<String>>,
    post_run_timeout_sec: Option<u64>,
    normalize_plan: Option<bool>,
    normalize_retry_count: Option<u32>,
    normalize_prompt_path: Option<PathBuf>,
    validate_plan_text: Option<bool>,
    min_plan_text_len: Option<u64>,
    run_summary_path: Option<PathBuf>,
    max_consecutive_task_failures: Option<u32>,
}

impl PealConfig {
    /// Load configuration with precedence: CLI > env > file > defaults.
    ///
    /// `config_path` — optional path to a TOML config file.
    /// `cli_args`    — values provided on the command line.
    pub fn load(config_path: Option<&Path>, cli_args: &RunArgs) -> anyhow::Result<Self> {
        Self::load_with_env(config_path, cli_args, real_env_var)
    }

    /// Validate that resolved paths satisfy filesystem requirements:
    /// plan_path must exist and be a regular file; repo_path must exist and
    /// be a directory.
    pub fn validate(&self) -> Result<(), crate::error::PealError> {
        if !self.plan_path.exists() {
            return Err(crate::error::PealError::PlanFileNotFound {
                path: self.plan_path.clone(),
            });
        }
        if !self.plan_path.is_file() {
            return Err(crate::error::PealError::InvalidPlanFile {
                path: self.plan_path.clone(),
            });
        }
        if !self.repo_path.exists() {
            return Err(crate::error::PealError::RepoPathNotFound {
                path: self.repo_path.clone(),
            });
        }
        if !self.repo_path.is_dir() {
            return Err(crate::error::PealError::RepoNotDirectory {
                path: self.repo_path.clone(),
            });
        }
        if !is_git_repo(&self.repo_path) {
            return Err(crate::error::PealError::RepoNotGitRepo {
                path: self.repo_path.clone(),
            });
        }
        if self.on_findings_remaining != "fail" && self.on_findings_remaining != "warn" {
            return Err(crate::error::PealError::InvalidOnFindingsRemaining {
                value: self.on_findings_remaining.clone(),
            });
        }
        if self.on_stet_fail != "fail"
            && self.on_stet_fail != "retry_once"
            && self.on_stet_fail != "skip"
        {
            return Err(crate::error::PealError::InvalidOnStetFail {
                value: self.on_stet_fail.clone(),
            });
        }
        Ok(())
    }

    /// Internal constructor that accepts an env-var lookup function,
    /// enabling deterministic testing without process-global mutation.
    fn load_with_env(
        config_path: Option<&Path>,
        cli_args: &RunArgs,
        env_fn: fn(&str) -> Option<String>,
    ) -> anyhow::Result<Self> {
        let file_layer = match config_path {
            Some(path) => load_file_layer(path)?,
            None => ConfigLayer::default(),
        };
        let env_layer = load_env_layer(env_fn)?;
        let cli_layer = cli_layer_from(cli_args);

        let merged = merge_layers(file_layer, env_layer, cli_layer);

        let plan_path = merged.plan_path.ok_or_else(|| {
            anyhow::anyhow!("plan_path is required (via --plan, PEAL_PLAN_PATH, or config file)")
        })?;
        let repo_path = merged.repo_path.ok_or_else(|| {
            anyhow::anyhow!("repo_path is required (via --repo, PEAL_REPO_PATH, or config file)")
        })?;

        Ok(PealConfig {
            agent_cmd: merged
                .agent_cmd
                .unwrap_or_else(|| DEFAULT_AGENT_CMD.to_owned()),
            plan_path,
            repo_path,
            stet_commands: merged.stet_commands.unwrap_or_default(),
            sandbox: merged.sandbox.unwrap_or_else(|| DEFAULT_SANDBOX.to_owned()),
            model: merged.model,
            max_address_rounds: merged
                .max_address_rounds
                .unwrap_or(DEFAULT_MAX_ADDRESS_ROUNDS),
            on_findings_remaining: merged
                .on_findings_remaining
                .unwrap_or_else(|| DEFAULT_ON_FINDINGS_REMAINING.to_owned()),
            state_dir: merged
                .state_dir
                .unwrap_or_else(|| PathBuf::from(DEFAULT_STATE_DIR)),
            phase_timeout_sec: merged
                .phase_timeout_sec
                .unwrap_or(DEFAULT_PHASE_TIMEOUT_SEC),
            phase_retry_count: merged
                .phase_retry_count
                .unwrap_or(DEFAULT_PHASE_RETRY_COUNT),
            phase_3_retry_count: merged
                .phase_3_retry_count
                .unwrap_or(DEFAULT_PHASE_3_RETRY_COUNT),
            parallel: {
                if merged.max_parallel == Some(0) {
                    false
                } else {
                    merged
                        .parallel
                        .or_else(|| merged.max_parallel.and_then(|n| if n > 0 { Some(true) } else { None }))
                        .unwrap_or(false)
                }
            },
            max_parallel: merged.max_parallel.unwrap_or(DEFAULT_MAX_PARALLEL),
            continue_with_remaining_tasks: merged.continue_with_remaining_tasks.unwrap_or(false),
            log_level: merged.log_level,
            log_file: merged.log_file,
        stet_path: merged.stet_path,
        stet_start_ref: merged.stet_start_ref,
        stet_start_extra_args: merged
            .stet_start_extra_args
            .unwrap_or_default(),
        stet_run_extra_args: merged.stet_run_extra_args.unwrap_or_default(),
        stet_disable_llm_triage: merged.stet_disable_llm_triage.unwrap_or(false),
        stet_dismiss_patterns: validate_stet_dismiss_patterns(
            merged.stet_dismiss_patterns.unwrap_or_default(),
        )
        .map_err(|e| anyhow::anyhow!("{}", e))?,
        on_stet_fail: merged
            .on_stet_fail
            .unwrap_or_else(|| DEFAULT_ON_STET_FAIL.to_owned()),
        post_run_commands: merged.post_run_commands.unwrap_or_default(),
        post_run_timeout_sec: merged.post_run_timeout_sec,
        normalize_plan: merged.normalize_plan.unwrap_or(false),
        normalize_retry_count: merged
            .normalize_retry_count
            .unwrap_or(DEFAULT_NORMALIZE_RETRY_COUNT),
        normalize_prompt_path: merged.normalize_prompt_path,
        validate_plan_text: merged.validate_plan_text.unwrap_or(false),
        min_plan_text_len: merged
            .min_plan_text_len
            .and_then(|u| u.try_into().ok()),
        run_summary_path: merged.run_summary_path,
        max_consecutive_task_failures: merged.max_consecutive_task_failures,
    })
    }
}

fn load_file_layer(path: &Path) -> anyhow::Result<ConfigLayer> {
    let contents = fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read config file {}: {e}", path.display()))?;
    let fc: FileConfig = toml::from_str(&contents)
        .map_err(|e| anyhow::anyhow!("failed to parse config file {}: {e}", path.display()))?;
    Ok(ConfigLayer {
        agent_cmd: fc.agent_cmd,
        plan_path: fc.plan_path,
        repo_path: fc.repo_path,
        stet_commands: fc.stet_commands,
        sandbox: fc.sandbox,
        model: fc.model,
        max_address_rounds: fc.max_address_rounds,
        on_findings_remaining: fc.on_findings_remaining,
        state_dir: fc.state_dir,
        phase_timeout_sec: fc.phase_timeout_sec,
        phase_retry_count: fc.phase_retry_count,
        phase_3_retry_count: fc.phase_3_retry_count,
        parallel: fc.parallel,
        max_parallel: fc.max_parallel,
        continue_with_remaining_tasks: fc.continue_with_remaining_tasks,
        log_level: fc.log_level,
        log_file: fc.log_file,
        stet_path: fc.stet_path,
        stet_start_ref: fc.stet_start_ref,
        stet_start_extra_args: fc.stet_start_extra_args,
        stet_run_extra_args: fc.stet_run_extra_args,
        stet_disable_llm_triage: fc.stet_disable_llm_triage,
        stet_dismiss_patterns: fc.stet_dismiss_patterns,
        on_stet_fail: fc.on_stet_fail,
        post_run_commands: fc.post_run_commands,
        post_run_timeout_sec: fc.post_run_timeout_sec,
        normalize_plan: fc.normalize_plan,
        normalize_retry_count: fc.normalize_retry_count,
        normalize_prompt_path: fc.normalize_prompt_path,
        validate_plan_text: fc.validate_plan_text,
        min_plan_text_len: fc.min_plan_text_len,
        run_summary_path: fc.run_summary_path,
        max_consecutive_task_failures: fc.max_consecutive_task_failures,
    })
}

/// Returns true if `path` is the root of a git worktree (or inside one).
fn is_git_repo(path: &Path) -> bool {
    let output = match Command::new("git")
        .arg("-C")
        .arg(path)
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .output()
    {
        Ok(o) => o,
        Err(_) => return false,
    };
    if !output.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.trim() == "true"
}

fn real_env_var(suffix: &str) -> Option<String> {
    let key = format!("{ENV_PREFIX}{suffix}");
    env::var(&key).ok().filter(|v| !v.is_empty())
}

fn load_env_layer(
    env_fn: fn(&str) -> Option<String>,
) -> Result<ConfigLayer, crate::error::PealError> {
    Ok(ConfigLayer {
        agent_cmd: env_fn("AGENT_CMD"),
        plan_path: env_fn("PLAN_PATH").map(PathBuf::from),
        repo_path: env_fn("REPO_PATH").map(PathBuf::from),
        stet_commands: env_fn("STET_COMMANDS")
            .map(|s| s.split(',').map(|c| c.trim().to_owned()).collect()),
        sandbox: env_fn("SANDBOX"),
        model: env_fn("MODEL"),
        max_address_rounds: parse_env_u32(env_fn, "MAX_ADDRESS_ROUNDS")?,
        on_findings_remaining: env_fn("ON_FINDINGS_REMAINING"),
        state_dir: env_fn("STATE_DIR").map(PathBuf::from),
        phase_timeout_sec: parse_env_u64(env_fn, "PHASE_TIMEOUT_SEC")?,
        phase_retry_count: parse_env_u32(env_fn, "PHASE_RETRY_COUNT")?,
        phase_3_retry_count: parse_env_u32(env_fn, "PHASE_3_RETRY_COUNT")?,
        parallel: parse_env_bool(env_fn, "PARALLEL")?,
        max_parallel: parse_env_u32(env_fn, "MAX_PARALLEL")?,
        continue_with_remaining_tasks: parse_env_bool(env_fn, "CONTINUE_WITH_REMAINING_TASKS")?,
        log_level: env_fn("LOG_LEVEL"),
        log_file: env_fn("LOG_FILE").map(PathBuf::from),
        stet_path: env_fn("STET_PATH").map(PathBuf::from),
        stet_start_ref: env_fn("STET_START_REF"),
        stet_start_extra_args: env_fn("STET_START_EXTRA_ARGS")
            .as_deref()
            .map(parse_extra_args_str),
        stet_run_extra_args: env_fn("STET_RUN_EXTRA_ARGS")
            .as_deref()
            .map(parse_extra_args_str),
        stet_disable_llm_triage: parse_env_bool(env_fn, "STET_DISABLE_LLM_TRIAGE")?,
        stet_dismiss_patterns: env_fn("STET_DISMISS_PATTERNS").as_deref().map(parse_dismiss_patterns),
        on_stet_fail: env_fn("ON_STET_FAIL"),
        post_run_commands: env_fn("POST_RUN_COMMANDS")
            .map(|s| s.split(',').map(|c| c.trim().to_owned()).filter(|c| !c.is_empty()).collect()),
        post_run_timeout_sec: parse_env_u64(env_fn, "POST_RUN_TIMEOUT_SEC")?,
        normalize_plan: parse_env_bool(env_fn, "NORMALIZE_PLAN")?,
        normalize_retry_count: parse_env_u32(env_fn, "NORMALIZE_RETRY_COUNT")?,
        normalize_prompt_path: env_fn("NORMALIZE_PROMPT_PATH").map(PathBuf::from),
        validate_plan_text: parse_env_bool(env_fn, "VALIDATE_PLAN_TEXT")?,
        min_plan_text_len: parse_env_u64(env_fn, "MIN_PLAN_TEXT_LEN")?,
        run_summary_path: env_fn("RUN_SUMMARY_PATH").map(PathBuf::from),
        max_consecutive_task_failures: parse_env_u32(env_fn, "MAX_CONSECUTIVE_TASK_FAILURES")?,
    })
}

fn parse_extra_args_str(s: &str) -> Vec<String> {
    s.split(',')
        .flat_map(|part| part.split_whitespace().map(|t| t.to_owned()))
        .filter(|t| !t.is_empty())
        .collect()
}

/// Parse PEAL_STET_DISMISS_PATTERNS: comma-separated "pattern|reason" (e.g. "generated|out_of_scope,false positive|false_positive").
/// Skips invalid reasons; returns empty vec if any entry fails to parse.
fn parse_dismiss_patterns(s: &str) -> Vec<StetDismissPattern> {
    s.split(',')
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .filter_map(|part| {
            let mut split = part.splitn(2, '|');
            let pattern = split.next()?.trim().to_owned();
            let reason = split.next()?.trim().to_owned();
            if pattern.is_empty() || !StetDismissPattern::is_valid_reason(&reason) {
                return None;
            }
            Some(StetDismissPattern { pattern, reason })
        })
        .collect()
}

/// Validate each pattern's reason; return error if any reason is not one of the four allowed.
fn validate_stet_dismiss_patterns(
    patterns: Vec<StetDismissPattern>,
) -> Result<Vec<StetDismissPattern>, PealError> {
    for p in &patterns {
        if !StetDismissPattern::is_valid_reason(&p.reason) {
            return Err(PealError::InvalidStetDismissReason {
                value: p.reason.clone(),
            });
        }
    }
    Ok(patterns)
}

fn parse_env_u32(
    env_fn: fn(&str) -> Option<String>,
    suffix: &str,
) -> Result<Option<u32>, crate::error::PealError> {
    match env_fn(suffix) {
        Some(s) => {
            s.parse::<u32>()
                .map(Some)
                .map_err(|e| crate::error::PealError::ConfigEnvParseError {
                    var: format!("{ENV_PREFIX}{suffix}"),
                    detail: e.to_string(),
                })
        }
        None => Ok(None),
    }
}

fn parse_env_u64(
    env_fn: fn(&str) -> Option<String>,
    suffix: &str,
) -> Result<Option<u64>, crate::error::PealError> {
    match env_fn(suffix) {
        Some(s) => {
            s.parse::<u64>()
                .map(Some)
                .map_err(|e| crate::error::PealError::ConfigEnvParseError {
                    var: format!("{ENV_PREFIX}{suffix}"),
                    detail: e.to_string(),
                })
        }
        None => Ok(None),
    }
}

fn parse_env_bool(
    env_fn: fn(&str) -> Option<String>,
    suffix: &str,
) -> Result<Option<bool>, crate::error::PealError> {
    match env_fn(suffix) {
        Some(s) => {
            s.parse::<bool>()
                .map(Some)
                .map_err(|e| crate::error::PealError::ConfigEnvParseError {
                    var: format!("{ENV_PREFIX}{suffix}"),
                    detail: e.to_string(),
                })
        }
        None => Ok(None),
    }
}

fn cli_layer_from(args: &RunArgs) -> ConfigLayer {
    ConfigLayer {
        plan_path: args.plan.clone(),
        repo_path: args.repo.clone(),
        agent_cmd: args.agent_cmd.clone(),
        model: args.model.clone(),
        sandbox: args.sandbox.clone(),
        state_dir: args.state_dir.clone(),
        phase_timeout_sec: args.phase_timeout_sec,
        phase_retry_count: args.phase_retry_count,
        phase_3_retry_count: args.phase_3_retry_count,
        parallel: if args.parallel { Some(true) } else { None },
        max_parallel: args.max_parallel,
        continue_with_remaining_tasks: if args.continue_with_remaining_tasks {
            Some(true)
        } else {
            None
        },
        max_address_rounds: args.max_address_rounds,
        on_findings_remaining: args.on_findings_remaining.clone(),
        stet_commands: None,
        log_level: args.log_level.clone(),
        log_file: args.log_file.clone(),
        stet_path: args.stet_path.clone(),
        stet_start_ref: args.stet_start_ref.clone(),
        stet_start_extra_args: args
            .stet_start_args
            .as_deref()
            .map(parse_extra_args_str),
        stet_run_extra_args: args.stet_run_args.as_deref().map(parse_extra_args_str),
        stet_disable_llm_triage: args.stet_disable_llm_triage,
        stet_dismiss_patterns: None,
        on_stet_fail: args.on_stet_fail.clone(),
        post_run_commands: args
            .post_run_commands
            .as_deref()
            .map(|s| s.split(',').map(|c| c.trim().to_owned()).filter(|c| !c.is_empty()).collect()),
        post_run_timeout_sec: args.post_run_timeout_sec,
        normalize_plan: if args.normalize { Some(true) } else { None },
        normalize_retry_count: args.normalize_retry_count,
        normalize_prompt_path: None,
        validate_plan_text: if args.validate_plan_text { Some(true) } else { None },
        min_plan_text_len: args.min_plan_text_len,
        run_summary_path: args.run_summary_path.clone(),
        max_consecutive_task_failures: args.max_consecutive_task_failures,
    }
}

/// Merge three layers. For each field, pick CLI first, then env, then file.
fn merge_layers(file: ConfigLayer, env: ConfigLayer, cli: ConfigLayer) -> ConfigLayer {
    ConfigLayer {
        agent_cmd: cli.agent_cmd.or(env.agent_cmd).or(file.agent_cmd),
        plan_path: cli.plan_path.or(env.plan_path).or(file.plan_path),
        repo_path: cli.repo_path.or(env.repo_path).or(file.repo_path),
        stet_commands: cli
            .stet_commands
            .or(env.stet_commands)
            .or(file.stet_commands),
        sandbox: cli.sandbox.or(env.sandbox).or(file.sandbox),
        model: cli.model.or(env.model).or(file.model),
        max_address_rounds: cli
            .max_address_rounds
            .or(env.max_address_rounds)
            .or(file.max_address_rounds),
        on_findings_remaining: cli
            .on_findings_remaining
            .or(env.on_findings_remaining)
            .or(file.on_findings_remaining),
        state_dir: cli.state_dir.or(env.state_dir).or(file.state_dir),
        phase_timeout_sec: cli
            .phase_timeout_sec
            .or(env.phase_timeout_sec)
            .or(file.phase_timeout_sec),
        phase_retry_count: cli
            .phase_retry_count
            .or(env.phase_retry_count)
            .or(file.phase_retry_count),
        phase_3_retry_count: cli
            .phase_3_retry_count
            .or(env.phase_3_retry_count)
            .or(file.phase_3_retry_count),
        parallel: cli.parallel.or(env.parallel).or(file.parallel),
        max_parallel: cli.max_parallel.or(env.max_parallel).or(file.max_parallel),
        continue_with_remaining_tasks: cli
            .continue_with_remaining_tasks
            .or(env.continue_with_remaining_tasks)
            .or(file.continue_with_remaining_tasks),
        log_level: cli.log_level.or(env.log_level).or(file.log_level),
        log_file: cli.log_file.or(env.log_file).or(file.log_file),
        stet_path: cli.stet_path.or(env.stet_path).or(file.stet_path),
        stet_start_ref: cli
            .stet_start_ref
            .or(env.stet_start_ref)
            .or(file.stet_start_ref),
        stet_start_extra_args: cli
            .stet_start_extra_args
            .or(env.stet_start_extra_args)
            .or(file.stet_start_extra_args),
        stet_run_extra_args: cli
            .stet_run_extra_args
            .or(env.stet_run_extra_args)
            .or(file.stet_run_extra_args),
        stet_disable_llm_triage: cli
            .stet_disable_llm_triage
            .or(env.stet_disable_llm_triage)
            .or(file.stet_disable_llm_triage),
        stet_dismiss_patterns: cli
            .stet_dismiss_patterns
            .or(env.stet_dismiss_patterns)
            .or(file.stet_dismiss_patterns),
        on_stet_fail: cli.on_stet_fail.or(env.on_stet_fail).or(file.on_stet_fail),
        post_run_commands: cli
            .post_run_commands
            .or(env.post_run_commands)
            .or(file.post_run_commands),
        post_run_timeout_sec: cli
            .post_run_timeout_sec
            .or(env.post_run_timeout_sec)
            .or(file.post_run_timeout_sec),
        normalize_plan: cli.normalize_plan.or(env.normalize_plan).or(file.normalize_plan),
        normalize_retry_count: cli
            .normalize_retry_count
            .or(env.normalize_retry_count)
            .or(file.normalize_retry_count),
        normalize_prompt_path: cli
            .normalize_prompt_path
            .or(env.normalize_prompt_path)
            .or(file.normalize_prompt_path),
        validate_plan_text: cli
            .validate_plan_text
            .or(env.validate_plan_text)
            .or(file.validate_plan_text),
        min_plan_text_len: cli
            .min_plan_text_len
            .or(env.min_plan_text_len)
            .or(file.min_plan_text_len),
        run_summary_path: cli
            .run_summary_path
            .or(env.run_summary_path)
            .or(file.run_summary_path),
        max_consecutive_task_failures: cli
            .max_consecutive_task_failures
            .or(env.max_consecutive_task_failures)
            .or(file.max_consecutive_task_failures),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_suffix: &str) -> Option<String> {
        None
    }

    fn minimal_cli_args(plan: Option<PathBuf>, repo: Option<PathBuf>) -> RunArgs {
        RunArgs {
            plan,
            repo,
            normalize: false,
            normalize_retry_count: None,
            config: None,
            agent_cmd: None,
            model: None,
            sandbox: None,
            state_dir: None,
            phase_timeout_sec: None,
            phase_retry_count: None,
            phase_3_retry_count: None,
            parallel: false,
            max_parallel: None,
            continue_with_remaining_tasks: false,
            max_address_rounds: None,
            on_findings_remaining: None,
            on_stet_fail: None,
            task: None,
            from_task: None,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
            stet_start_args: None,
            stet_run_args: None,
            stet_disable_llm_triage: None,
            post_run_commands: None,
            post_run_timeout_sec: None,
            validate_plan_text: false,
            min_plan_text_len: None,
            run_summary_path: None,
            max_consecutive_task_failures: None,
        }
    }

    #[test]
    fn defaults_applied_when_only_required_fields_present() {
        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/repo")));
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();

        assert_eq!(cfg.agent_cmd, "agent");
        assert_eq!(cfg.plan_path, PathBuf::from("p.md"));
        assert_eq!(cfg.repo_path, PathBuf::from("/repo"));
        assert!(cfg.stet_commands.is_empty());
        assert_eq!(cfg.sandbox, "disabled");
        assert_eq!(cfg.model, None);
        assert_eq!(cfg.max_address_rounds, 5);
        assert_eq!(cfg.on_findings_remaining, "fail");
        assert_eq!(cfg.state_dir, PathBuf::from(".peal"));
        assert_eq!(cfg.phase_timeout_sec, 1800);
        assert!(!cfg.parallel);
        assert_eq!(cfg.max_parallel, 4);
    }

    #[test]
    fn validate_plan_text_defaults_off() {
        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();
        assert!(!cfg.validate_plan_text);
        assert_eq!(cfg.min_plan_text_len, None);
    }

    #[test]
    fn validate_plan_text_and_min_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            r#"
plan_path = "p.md"
repo_path = "/r"
validate_plan_text = true
min_plan_text_len = 500
"#,
        )
        .unwrap();

        let args = minimal_cli_args(None, None);
        let cfg = PealConfig::load_with_env(Some(&cfg_path), &args, no_env).unwrap();
        assert!(cfg.validate_plan_text);
        assert_eq!(cfg.min_plan_text_len, Some(500));
    }

    #[test]
    fn validate_plan_text_from_env() {
        fn fake_env(suffix: &str) -> Option<String> {
            match suffix {
                "VALIDATE_PLAN_TEXT" => Some("true".to_owned()),
                "MIN_PLAN_TEXT_LEN" => Some("100".to_owned()),
                _ => None,
            }
        }

        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let cfg = PealConfig::load_with_env(None, &args, fake_env).unwrap();
        assert!(cfg.validate_plan_text);
        assert_eq!(cfg.min_plan_text_len, Some(100));
    }

    #[test]
    fn validate_plan_text_from_cli() {
        let mut args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        args.validate_plan_text = true;
        args.min_plan_text_len = Some(200);
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();
        assert!(cfg.validate_plan_text);
        assert_eq!(cfg.min_plan_text_len, Some(200));
    }

    #[test]
    fn documented_example_toml_parses_successfully() {
        // Matches docs/configuration.md "Full TOML example" to avoid drift.
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            r#"
# Required (or provide via --plan / --repo or PEAL_PLAN_PATH / PEAL_REPO_PATH)
plan_path = "plans/my-plan.md"
repo_path = "/path/to/repo"

# Optional: Cursor CLI and execution
agent_cmd = "agent"
state_dir = ".peal"
phase_timeout_sec = 1800
parallel = true
max_parallel = 4
on_findings_remaining = "fail"

# Optional: stet integration
stet_commands = ["stet start HEAD~1", "stet run"]
stet_start_extra_args = ["--allow-dirty"]
stet_run_extra_args = ["--verify", "--context", "256k"]
stet_disable_llm_triage = false
stet_dismiss_patterns = [
  { pattern = "generated", reason = "out_of_scope" },
  { pattern = "false positive", reason = "false_positive" }
]

# Optional: post-run commands (e.g. stet finish)
post_run_commands = ["stet finish", "echo done"]
post_run_timeout_sec = 60
"#,
        )
        .unwrap();

        let args = minimal_cli_args(None, None);
        let cfg = PealConfig::load_with_env(Some(&cfg_path), &args, no_env).unwrap();

        assert_eq!(cfg.plan_path, PathBuf::from("plans/my-plan.md"));
        assert_eq!(cfg.repo_path, PathBuf::from("/path/to/repo"));
        assert_eq!(cfg.agent_cmd, "agent");
        assert_eq!(cfg.state_dir, PathBuf::from(".peal"));
        assert_eq!(cfg.phase_timeout_sec, 1800);
        assert!(cfg.parallel);
        assert_eq!(cfg.max_parallel, 4);
        assert_eq!(cfg.on_findings_remaining, "fail");
        assert_eq!(
            cfg.stet_commands,
            vec!["stet start HEAD~1", "stet run"]
        );
        assert_eq!(cfg.stet_start_extra_args, vec!["--allow-dirty"]);
        assert_eq!(
            cfg.stet_run_extra_args,
            vec!["--verify", "--context", "256k"]
        );
        assert!(!cfg.stet_disable_llm_triage);
        assert_eq!(cfg.stet_dismiss_patterns.len(), 2);
        assert_eq!(cfg.stet_dismiss_patterns[0].pattern, "generated");
        assert_eq!(cfg.stet_dismiss_patterns[0].reason, "out_of_scope");
        assert_eq!(cfg.stet_dismiss_patterns[1].pattern, "false positive");
        assert_eq!(cfg.stet_dismiss_patterns[1].reason, "false_positive");
        assert_eq!(
            cfg.post_run_commands,
            vec!["stet finish", "echo done"]
        );
        assert_eq!(cfg.post_run_timeout_sec, Some(60));
    }

    #[test]
    fn missing_plan_path_errors() {
        let args = minimal_cli_args(None, Some(PathBuf::from("/repo")));
        let err = PealConfig::load_with_env(None, &args, no_env).unwrap_err();
        assert!(
            format!("{err}").contains("plan_path is required"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn missing_repo_path_errors() {
        let args = minimal_cli_args(Some(PathBuf::from("p.md")), None);
        let err = PealConfig::load_with_env(None, &args, no_env).unwrap_err();
        assert!(
            format!("{err}").contains("repo_path is required"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn loads_from_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            r#"
agent_cmd = "cursor-agent"
plan_path = "my-plan.md"
repo_path = "/my/repo"
stet_commands = ["stet start HEAD~1", "stet run"]
sandbox = "enabled"
model = "gpt-5.2"
max_address_rounds = 5
state_dir = ".my-state"
phase_timeout_sec = 600
parallel = true
max_parallel = 8
"#,
        )
        .unwrap();

        let args = minimal_cli_args(None, None);
        let cfg = PealConfig::load_with_env(Some(&cfg_path), &args, no_env).unwrap();

        assert_eq!(cfg.agent_cmd, "cursor-agent");
        assert_eq!(cfg.plan_path, PathBuf::from("my-plan.md"));
        assert_eq!(cfg.repo_path, PathBuf::from("/my/repo"));
        assert_eq!(cfg.stet_commands, vec!["stet start HEAD~1", "stet run"]);
        assert_eq!(cfg.sandbox, "enabled");
        assert_eq!(cfg.model.as_deref(), Some("gpt-5.2"));
        assert_eq!(cfg.max_address_rounds, 5);
        assert_eq!(cfg.state_dir, PathBuf::from(".my-state"));
        assert_eq!(cfg.phase_timeout_sec, 600);
        assert!(cfg.parallel);
        assert_eq!(cfg.max_parallel, 8);
    }

    #[test]
    fn cli_overrides_file() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            r#"
plan_path = "file-plan.md"
repo_path = "/file/repo"
model = "from-file"
"#,
        )
        .unwrap();

        let args = RunArgs {
            plan: Some(PathBuf::from("cli-plan.md")),
            repo: None,
            normalize: false,
            normalize_retry_count: None,
            config: None,
            agent_cmd: None,
            model: Some("from-cli".to_owned()),
            sandbox: None,
            state_dir: None,
            phase_timeout_sec: None,
            phase_retry_count: None,
            phase_3_retry_count: None,
            parallel: false,
            max_parallel: None,
            continue_with_remaining_tasks: false,
            max_address_rounds: None,
            on_findings_remaining: None,
            on_stet_fail: None,
            task: None,
            from_task: None,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
            stet_start_args: None,
            stet_run_args: None,
            stet_disable_llm_triage: None,
            post_run_commands: None,
            post_run_timeout_sec: None,
            validate_plan_text: false,
            min_plan_text_len: None,
            run_summary_path: None,
            max_consecutive_task_failures: None,
        };
        let cfg = PealConfig::load_with_env(Some(&cfg_path), &args, no_env).unwrap();

        assert_eq!(cfg.plan_path, PathBuf::from("cli-plan.md"), "CLI wins");
        assert_eq!(cfg.repo_path, PathBuf::from("/file/repo"), "file fallback");
        assert_eq!(cfg.model.as_deref(), Some("from-cli"), "CLI wins");
    }

    #[test]
    fn env_overrides_file() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            r#"
plan_path = "file-plan.md"
repo_path = "/file/repo"
agent_cmd = "from-file"
"#,
        )
        .unwrap();

        fn fake_env(suffix: &str) -> Option<String> {
            if suffix == "AGENT_CMD" {
                Some("from-env".to_owned())
            } else {
                None
            }
        }

        let args = minimal_cli_args(None, None);
        let cfg = PealConfig::load_with_env(Some(&cfg_path), &args, fake_env).unwrap();

        assert_eq!(cfg.agent_cmd, "from-env", "env wins over file");
    }

    #[test]
    fn cli_overrides_env() {
        fn fake_env(suffix: &str) -> Option<String> {
            if suffix == "AGENT_CMD" {
                Some("from-env".to_owned())
            } else {
                None
            }
        }

        let args = RunArgs {
            plan: Some(PathBuf::from("p.md")),
            repo: Some(PathBuf::from("/r")),
            normalize: false,
            normalize_retry_count: None,
            config: None,
            agent_cmd: Some("from-cli".to_owned()),
            model: None,
            sandbox: None,
            state_dir: None,
            phase_timeout_sec: None,
            phase_retry_count: None,
            phase_3_retry_count: None,
            parallel: false,
            max_parallel: None,
            continue_with_remaining_tasks: false,
            max_address_rounds: None,
            on_findings_remaining: None,
            on_stet_fail: None,
            task: None,
            from_task: None,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
            stet_start_args: None,
            stet_run_args: None,
            stet_disable_llm_triage: None,
            post_run_commands: None,
            post_run_timeout_sec: None,
            validate_plan_text: false,
            min_plan_text_len: None,
            run_summary_path: None,
            max_consecutive_task_failures: None,
        };
        let cfg = PealConfig::load_with_env(None, &args, fake_env).unwrap();

        assert_eq!(cfg.agent_cmd, "from-cli", "CLI wins over env");
    }

    #[test]
    fn invalid_toml_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("peal.toml");
        fs::write(&cfg_path, "not valid {{{{ toml").unwrap();

        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let err = PealConfig::load_with_env(Some(&cfg_path), &args, no_env).unwrap_err();
        assert!(
            format!("{err}").contains("failed to parse config file"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn unknown_toml_key_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            r#"
plan_path = "p.md"
repo_path = "/r"
bogus_key = true
"#,
        )
        .unwrap();

        let args = minimal_cli_args(None, None);
        let err = PealConfig::load_with_env(Some(&cfg_path), &args, no_env).unwrap_err();
        assert!(
            format!("{err}").contains("failed to parse config file"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn missing_config_file_returns_error() {
        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let err = PealConfig::load_with_env(Some(Path::new("/no/such/file.toml")), &args, no_env)
            .unwrap_err();
        assert!(
            format!("{err}").contains("failed to read config file"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn env_stet_commands_parsed_from_comma_separated() {
        fn fake_env(suffix: &str) -> Option<String> {
            if suffix == "STET_COMMANDS" {
                Some("stet start HEAD~1, stet run".to_owned())
            } else {
                None
            }
        }

        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let cfg = PealConfig::load_with_env(None, &args, fake_env).unwrap();

        assert_eq!(cfg.stet_commands, vec!["stet start HEAD~1", "stet run"]);
    }

    #[test]
    fn parallel_flag_from_cli() {
        let args = RunArgs {
            plan: Some(PathBuf::from("p.md")),
            repo: Some(PathBuf::from("/r")),
            normalize: false,
            normalize_retry_count: None,
            config: None,
            agent_cmd: None,
            model: None,
            sandbox: None,
            state_dir: None,
            phase_timeout_sec: None,
            phase_retry_count: None,
            phase_3_retry_count: None,
            parallel: false,
            max_parallel: Some(2),
            continue_with_remaining_tasks: false,
            max_address_rounds: None,
            on_findings_remaining: None,
            on_stet_fail: None,
            task: None,
            from_task: None,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
            stet_start_args: None,
            stet_run_args: None,
            stet_disable_llm_triage: None,
            post_run_commands: None,
            post_run_timeout_sec: None,
            validate_plan_text: false,
            min_plan_text_len: None,
            run_summary_path: None,
            max_consecutive_task_failures: None,
        };
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();

        assert!(cfg.parallel);
        assert_eq!(cfg.max_parallel, 2);
    }

    // -- validate() tests --

    #[test]
    fn validate_succeeds_with_valid_file_and_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git").args(["init"]).current_dir(dir.path()).output().ok();
        let plan_path = dir.path().join("plan.md");
        fs::write(&plan_path, "## Task 1\nDo it.").unwrap();

        let args = minimal_cli_args(Some(plan_path), Some(dir.path().to_path_buf()));
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();

        cfg.validate().expect("should succeed with valid paths");
    }

    #[test]
    fn validate_fails_when_plan_path_does_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let args = minimal_cli_args(
            Some(dir.path().join("nonexistent.md")),
            Some(dir.path().to_path_buf()),
        );
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();

        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("Invalid or missing plan file."),
            "expected 'Invalid or missing plan file.', got: {msg}"
        );
    }

    #[test]
    fn validate_fails_when_plan_path_is_directory() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("subdir");
        fs::create_dir(&sub).unwrap();

        let args = minimal_cli_args(Some(sub), Some(dir.path().to_path_buf()));
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();

        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("Invalid or missing plan file."),
            "expected 'Invalid or missing plan file.', got: {msg}"
        );
    }

    #[test]
    fn validate_fails_when_repo_path_does_not_exist() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = dir.path().join("plan.md");
        fs::write(&plan_path, "## Task 1\nDo it.").unwrap();

        let args = minimal_cli_args(Some(plan_path), Some(PathBuf::from("/no/such/repo")));
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();

        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("does not exist"),
            "expected 'does not exist', got: {msg}"
        );
    }

    #[test]
    fn validate_fails_when_repo_path_is_file() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = dir.path().join("plan.md");
        fs::write(&plan_path, "## Task 1\nDo it.").unwrap();

        let fake_repo = dir.path().join("not-a-dir.txt");
        fs::write(&fake_repo, "I am a file").unwrap();

        let args = minimal_cli_args(Some(plan_path), Some(fake_repo));
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();

        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("not a directory"),
            "expected 'not a directory', got: {msg}"
        );
    }

    #[test]
    fn validate_fails_when_repo_not_git() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = dir.path().join("plan.md");
        fs::write(&plan_path, "## Task 1\nDo it.").unwrap();
        // Do not run git init; dir is not a git repo.

        let args = minimal_cli_args(Some(plan_path), Some(dir.path().to_path_buf()));
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();

        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("not a git repository"),
            "expected 'not a git repository', got: {msg}"
        );
    }

    #[test]
    fn validate_rejects_invalid_on_stet_fail() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git").args(["init"]).current_dir(dir.path()).output().ok();
        let plan_path = dir.path().join("plan.md");
        fs::write(&plan_path, "## Task 1\nDo it.").unwrap();

        let mut args = minimal_cli_args(Some(plan_path), Some(dir.path().to_path_buf()));
        args.on_stet_fail = Some("invalid".to_owned());
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();

        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("Invalid on_stet_fail"),
            "expected Invalid on_stet_fail, got: {msg}"
        );
    }

    #[test]
    fn partial_toml_fills_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            r#"
plan_path = "p.md"
repo_path = "/r"
"#,
        )
        .unwrap();

        let args = minimal_cli_args(None, None);
        let cfg = PealConfig::load_with_env(Some(&cfg_path), &args, no_env).unwrap();

        assert_eq!(cfg.agent_cmd, "agent");
        assert_eq!(cfg.sandbox, "disabled");
        assert_eq!(cfg.max_address_rounds, 5);
        assert_eq!(cfg.on_findings_remaining, "fail");
        assert_eq!(cfg.phase_timeout_sec, 1800);
        assert!(!cfg.parallel);
        assert_eq!(cfg.max_parallel, 4);
    }

    #[test]
    fn full_precedence_chain() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            r#"
plan_path = "file.md"
repo_path = "/file"
agent_cmd = "from-file"
model = "file-model"
sandbox = "file-sandbox"
"#,
        )
        .unwrap();

        fn fake_env(suffix: &str) -> Option<String> {
            match suffix {
                "AGENT_CMD" => Some("from-env".to_owned()),
                "MODEL" => Some("env-model".to_owned()),
                _ => None,
            }
        }

        let args = RunArgs {
            plan: None,
            repo: None,
            normalize: false,
            normalize_retry_count: None,
            config: None,
            agent_cmd: Some("from-cli".to_owned()),
            model: None,
            sandbox: None,
            state_dir: None,
            phase_timeout_sec: None,
            phase_retry_count: None,
            phase_3_retry_count: None,
            parallel: false,
            max_parallel: None,
            continue_with_remaining_tasks: false,
            max_address_rounds: None,
            on_findings_remaining: None,
            on_stet_fail: None,
            task: None,
            from_task: None,
            log_level: None,
            log_file: None,
            stet_path: None,
            stet_start_ref: None,
            stet_start_args: None,
            stet_run_args: None,
            stet_disable_llm_triage: None,
            post_run_commands: None,
            post_run_timeout_sec: None,
            validate_plan_text: false,
            min_plan_text_len: None,
            run_summary_path: None,
            max_consecutive_task_failures: None,
        };
        let cfg = PealConfig::load_with_env(Some(&cfg_path), &args, fake_env).unwrap();

        assert_eq!(cfg.agent_cmd, "from-cli", "CLI > env > file");
        assert_eq!(cfg.model.as_deref(), Some("env-model"), "env > file");
        assert_eq!(cfg.sandbox, "file-sandbox", "file used when no env/cli");
        assert_eq!(cfg.plan_path, PathBuf::from("file.md"), "file fallback");
    }

    #[test]
    fn invalid_env_var_returns_error() {
        fn fake_env(suffix: &str) -> Option<String> {
            if suffix == "MAX_PARALLEL" {
                Some("not-a-number".to_owned())
            } else {
                None
            }
        }

        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let err = PealConfig::load_with_env(None, &args, fake_env).unwrap_err();
        assert!(
            format!("{err}").contains("Failed to parse environment variable"),
            "unexpected: {err}"
        );
        assert!(
            format!("{err}").contains("PEAL_MAX_PARALLEL"),
            "should mention the variable name"
        );
    }

    #[test]
    fn on_findings_remaining_defaults_to_fail() {
        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();
        assert_eq!(cfg.on_findings_remaining, "fail");
    }

    #[test]
    fn on_findings_remaining_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            r#"
plan_path = "p.md"
repo_path = "/r"
on_findings_remaining = "warn"
"#,
        )
        .unwrap();

        let args = minimal_cli_args(None, None);
        let cfg = PealConfig::load_with_env(Some(&cfg_path), &args, no_env).unwrap();
        assert_eq!(cfg.on_findings_remaining, "warn");
    }

    #[test]
    fn on_findings_remaining_from_env() {
        fn fake_env(suffix: &str) -> Option<String> {
            if suffix == "ON_FINDINGS_REMAINING" {
                Some("warn".to_owned())
            } else {
                None
            }
        }

        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let cfg = PealConfig::load_with_env(None, &args, fake_env).unwrap();
        assert_eq!(cfg.on_findings_remaining, "warn");
    }

    #[test]
    fn on_findings_remaining_from_cli() {
        let mut args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        args.on_findings_remaining = Some("warn".to_owned());
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();
        assert_eq!(cfg.on_findings_remaining, "warn");
    }

    #[test]
    fn on_findings_remaining_cli_overrides_env() {
        fn fake_env(suffix: &str) -> Option<String> {
            if suffix == "ON_FINDINGS_REMAINING" {
                Some("warn".to_owned())
            } else {
                None
            }
        }

        let mut args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        args.on_findings_remaining = Some("fail".to_owned());
        let cfg = PealConfig::load_with_env(None, &args, fake_env).unwrap();
        assert_eq!(cfg.on_findings_remaining, "fail");
    }

    #[test]
    fn validate_rejects_invalid_on_findings_remaining() {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git").args(["init"]).current_dir(dir.path()).output().ok();
        let plan_path = dir.path().join("plan.md");
        fs::write(&plan_path, "## Task 1\nDo it.").unwrap();

        let mut args = minimal_cli_args(Some(plan_path), Some(dir.path().to_path_buf()));
        args.on_findings_remaining = Some("panic".to_owned());
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();

        let err = cfg.validate().unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("Invalid on_findings_remaining"),
            "expected InvalidOnFindingsRemaining, got: {msg}"
        );
    }

    #[test]
    fn stet_extra_args_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            r#"
plan_path = "p.md"
repo_path = "/r"
stet_start_extra_args = ["--allow-dirty"]
stet_run_extra_args = ["--verify", "--context", "256k"]
"#,
        )
        .unwrap();

        let args = minimal_cli_args(None, None);
        let cfg = PealConfig::load_with_env(Some(&cfg_path), &args, no_env).unwrap();

        assert_eq!(cfg.stet_start_extra_args, vec!["--allow-dirty"]);
        assert_eq!(
            cfg.stet_run_extra_args,
            vec!["--verify", "--context", "256k"]
        );
    }

    #[test]
    fn stet_extra_args_from_env() {
        fn fake_env(suffix: &str) -> Option<String> {
            match suffix {
                "STET_START_EXTRA_ARGS" => Some("--allow-dirty".to_owned()),
                "STET_RUN_EXTRA_ARGS" => Some("--verify --context 256k".to_owned()),
                _ => None,
            }
        }

        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let cfg = PealConfig::load_with_env(None, &args, fake_env).unwrap();

        assert_eq!(cfg.stet_start_extra_args, vec!["--allow-dirty"]);
        assert_eq!(
            cfg.stet_run_extra_args,
            vec!["--verify", "--context", "256k"]
        );
    }

    #[test]
    fn parse_extra_args_str_splits_space_and_comma() {
        assert_eq!(
            parse_extra_args_str("--a --b"),
            vec!["--a", "--b"]
        );
        assert_eq!(
            parse_extra_args_str("--a,--b,256k"),
            vec!["--a", "--b", "256k"]
        );
        assert_eq!(parse_extra_args_str(""), vec![] as Vec<String>);
    }

    #[test]
    fn post_run_commands_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            r#"
plan_path = "p.md"
repo_path = "/r"
post_run_commands = ["stet finish", "echo done"]
post_run_timeout_sec = 60
"#,
        )
        .unwrap();

        let args = minimal_cli_args(None, None);
        let cfg = PealConfig::load_with_env(Some(&cfg_path), &args, no_env).unwrap();

        assert_eq!(
            cfg.post_run_commands,
            vec!["stet finish", "echo done"]
        );
        assert_eq!(cfg.post_run_timeout_sec, Some(60));
    }

    #[test]
    fn post_run_commands_default_empty_and_no_timeout() {
        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();

        assert!(cfg.post_run_commands.is_empty());
        assert_eq!(cfg.post_run_timeout_sec, None);
    }

    #[test]
    fn normalize_plan_defaults_to_false() {
        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();
        assert!(!cfg.normalize_plan);
    }

    #[test]
    fn normalize_plan_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            r#"
plan_path = "p.md"
repo_path = "/r"
normalize_plan = true
"#,
        )
        .unwrap();

        let args = minimal_cli_args(None, None);
        let cfg = PealConfig::load_with_env(Some(&cfg_path), &args, no_env).unwrap();
        assert!(cfg.normalize_plan);
    }

    #[test]
    fn normalize_plan_from_env() {
        fn fake_env(suffix: &str) -> Option<String> {
            if suffix == "NORMALIZE_PLAN" {
                Some("true".to_owned())
            } else {
                None
            }
        }

        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let cfg = PealConfig::load_with_env(None, &args, fake_env).unwrap();
        assert!(cfg.normalize_plan);
    }

    #[test]
    fn normalize_plan_from_cli() {
        let mut args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        args.normalize = true;
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();
        assert!(cfg.normalize_plan);
    }

    #[test]
    fn normalize_plan_cli_overrides_env_and_file() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            r#"
plan_path = "p.md"
repo_path = "/r"
normalize_plan = true
"#,
        )
        .unwrap();

        fn fake_env(suffix: &str) -> Option<String> {
            if suffix == "NORMALIZE_PLAN" {
                Some("false".to_owned())
            } else {
                None
            }
        }

        let mut args = minimal_cli_args(None, None);
        args.plan = Some(PathBuf::from("p.md"));
        args.repo = Some(PathBuf::from("/r"));
        args.normalize = true;
        let cfg = PealConfig::load_with_env(Some(&cfg_path), &args, fake_env).unwrap();
        assert!(cfg.normalize_plan, "CLI --normalize wins over env and file");
    }

    #[test]
    fn normalize_retry_count_defaults_to_zero() {
        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();
        assert_eq!(cfg.normalize_retry_count, 0);
    }

    #[test]
    fn normalize_retry_count_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            r#"
plan_path = "p.md"
repo_path = "/r"
normalize_retry_count = 2
"#,
        )
        .unwrap();

        let args = minimal_cli_args(None, None);
        let cfg = PealConfig::load_with_env(Some(&cfg_path), &args, no_env).unwrap();
        assert_eq!(cfg.normalize_retry_count, 2);
    }

    #[test]
    fn normalize_retry_count_from_env() {
        fn fake_env(suffix: &str) -> Option<String> {
            if suffix == "NORMALIZE_RETRY_COUNT" {
                Some("3".to_owned())
            } else {
                None
            }
        }

        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let cfg = PealConfig::load_with_env(None, &args, fake_env).unwrap();
        assert_eq!(cfg.normalize_retry_count, 3);
    }

    #[test]
    fn phase_3_retry_count_defaults_to_zero() {
        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();
        assert_eq!(cfg.phase_3_retry_count, 0);
    }

    #[test]
    fn phase_3_retry_count_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            r#"
plan_path = "p.md"
repo_path = "/r"
phase_3_retry_count = 2
"#,
        )
        .unwrap();

        let args = minimal_cli_args(None, None);
        let cfg = PealConfig::load_with_env(Some(&cfg_path), &args, no_env).unwrap();
        assert_eq!(cfg.phase_3_retry_count, 2);
    }

    #[test]
    fn phase_3_retry_count_from_env() {
        fn fake_env(suffix: &str) -> Option<String> {
            if suffix == "PHASE_3_RETRY_COUNT" {
                Some("1".to_owned())
            } else {
                None
            }
        }

        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let cfg = PealConfig::load_with_env(None, &args, fake_env).unwrap();
        assert_eq!(cfg.phase_3_retry_count, 1);
    }

    #[test]
    fn phase_3_retry_count_from_cli() {
        let mut args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        args.phase_3_retry_count = Some(2);
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();
        assert_eq!(cfg.phase_3_retry_count, 2);
    }

    #[test]
    fn normalize_prompt_path_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let prompt_path = dir.path().join("custom-prompt.txt");
        fs::write(&prompt_path, "Convert: {{DOC}}").unwrap();
        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            format!(
                r#"
plan_path = "p.md"
repo_path = "/r"
normalize_prompt_path = "{}"
"#,
                prompt_path.display()
            ),
        )
        .unwrap();

        let args = minimal_cli_args(None, None);
        let cfg = PealConfig::load_with_env(Some(&cfg_path), &args, no_env).unwrap();
        assert_eq!(cfg.normalize_prompt_path.as_deref(), Some(prompt_path.as_path()));
    }

    #[test]
    fn normalize_prompt_path_from_env() {
        let dir = tempfile::tempdir().unwrap();
        let prompt_path = dir.path().join("env-prompt.txt");
        fs::write(&prompt_path, "{{DOC}}").unwrap();

        fn fake_env(suffix: &str) -> Option<String> {
            if suffix == "NORMALIZE_PROMPT_PATH" {
                Some(std::path::Path::new("/tmp/custom-prompt.txt").display().to_string())
            } else {
                None
            }
        }

        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let cfg = PealConfig::load_with_env(None, &args, fake_env).unwrap();
        assert_eq!(
            cfg.normalize_prompt_path.as_deref(),
            Some(std::path::Path::new("/tmp/custom-prompt.txt"))
        );
    }

    #[test]
    fn post_run_commands_from_env() {
        fn fake_env(suffix: &str) -> Option<String> {
            match suffix {
                "POST_RUN_COMMANDS" => Some("stet finish, echo done".to_owned()),
                "POST_RUN_TIMEOUT_SEC" => Some("120".to_owned()),
                _ => None,
            }
        }

        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let cfg = PealConfig::load_with_env(None, &args, fake_env).unwrap();

        assert_eq!(
            cfg.post_run_commands,
            vec!["stet finish", "echo done"]
        );
        assert_eq!(cfg.post_run_timeout_sec, Some(120));
    }

    #[test]
    fn run_summary_path_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let summary_path = dir.path().join("summary.json");
        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            format!(
                r#"
plan_path = "p.md"
repo_path = "/r"
run_summary_path = "{}"
"#,
                summary_path.display()
            ),
        )
        .unwrap();

        let args = minimal_cli_args(None, None);
        let cfg = PealConfig::load_with_env(Some(&cfg_path), &args, no_env).unwrap();
        assert_eq!(cfg.run_summary_path.as_deref(), Some(summary_path.as_path()));
    }

    #[test]
    fn run_summary_path_from_env() {
        fn fake_env(suffix: &str) -> Option<String> {
            if suffix == "RUN_SUMMARY_PATH" {
                Some("/tmp/peal-summary.json".to_owned())
            } else {
                None
            }
        }

        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let cfg = PealConfig::load_with_env(None, &args, fake_env).unwrap();
        assert_eq!(
            cfg.run_summary_path.as_deref(),
            Some(std::path::Path::new("/tmp/peal-summary.json"))
        );
    }

    #[test]
    fn run_summary_path_from_cli() {
        let dir = tempfile::tempdir().unwrap();
        let custom = dir.path().join("custom.json");
        let mut args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        args.run_summary_path = Some(custom.clone());
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();
        assert_eq!(cfg.run_summary_path.as_deref(), Some(custom.as_path()));
    }

    #[test]
    fn max_consecutive_task_failures_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            r#"
plan_path = "p.md"
repo_path = "/r"
max_consecutive_task_failures = 3
"#,
        )
        .unwrap();

        let args = minimal_cli_args(None, None);
        let cfg = PealConfig::load_with_env(Some(&cfg_path), &args, no_env).unwrap();
        assert_eq!(cfg.max_consecutive_task_failures, Some(3));
    }

    #[test]
    fn max_consecutive_task_failures_from_env() {
        fn fake_env(suffix: &str) -> Option<String> {
            if suffix == "MAX_CONSECUTIVE_TASK_FAILURES" {
                Some("5".to_owned())
            } else {
                None
            }
        }

        let args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        let cfg = PealConfig::load_with_env(None, &args, fake_env).unwrap();
        assert_eq!(cfg.max_consecutive_task_failures, Some(5));
    }

    #[test]
    fn max_consecutive_task_failures_from_cli() {
        let mut args = minimal_cli_args(Some(PathBuf::from("p.md")), Some(PathBuf::from("/r")));
        args.max_consecutive_task_failures = Some(2);
        let cfg = PealConfig::load_with_env(None, &args, no_env).unwrap();
        assert_eq!(cfg.max_consecutive_task_failures, Some(2));
    }

    #[test]
    fn max_consecutive_task_failures_precedence_cli_over_env_over_file() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("peal.toml");
        fs::write(
            &cfg_path,
            r#"
plan_path = "p.md"
repo_path = "/r"
max_consecutive_task_failures = 10
"#,
        )
        .unwrap();

        fn fake_env(suffix: &str) -> Option<String> {
            if suffix == "MAX_CONSECUTIVE_TASK_FAILURES" {
                Some("7".to_owned())
            } else {
                None
            }
        }

        let mut args = minimal_cli_args(None, None);
        args.max_consecutive_task_failures = Some(2);
        let cfg = PealConfig::load_with_env(Some(&cfg_path), &args, fake_env).unwrap();
        assert_eq!(cfg.max_consecutive_task_failures, Some(2), "CLI wins");
    }
}
