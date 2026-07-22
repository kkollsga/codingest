//! GitHub shallow-clone helper (ported from repo.py).

// Phase G.3-pre: returns `Arc<DirGraph>` to match builder::run_with_options.
// The pyapi callsite (`code_tree.repo_tree` pyfunction) wraps via
// `KnowledgeGraph::from_arc`.
use base64::Engine;
use kglite::api::DirGraph;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

#[allow(clippy::too_many_arguments)]
pub fn clone_and_build(
    repo: &str,
    save_to: Option<&Path>,
    clone_to: Option<&Path>,
    branch: Option<&str>,
    token: Option<&str>,
    verbose: bool,
    include_tests: bool,
    max_loc_per_file: Option<usize>,
    include_docs: bool,
) -> Result<Arc<DirGraph>, String> {
    if !repo.contains('/') || repo.matches('/').count() != 1 {
        return Err(format!(
            "repo must be in 'org/repo' format, got: {:?}",
            repo
        ));
    }

    let env_token: Option<String> = std::env::var("GITHUB_TOKEN").ok();
    let token = token.map(str::to_string).or(env_token);

    if let Some(parent) = clone_to {
        let repo_path = clone_repo(repo, parent, branch, token.as_deref(), verbose)?;
        return crate::builder::run_with_options(
            &repo_path,
            verbose,
            include_tests,
            save_to,
            max_loc_per_file,
            include_docs,
        );
    }

    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let repo_path = clone_repo(repo, tmp.path(), branch, token.as_deref(), verbose)?;
    crate::builder::run_with_options(
        &repo_path,
        verbose,
        include_tests,
        save_to,
        max_loc_per_file,
        include_docs,
    )
}

fn clone_repo(
    repo: &str,
    parent: &Path,
    branch: Option<&str>,
    token: Option<&str>,
    verbose: bool,
) -> Result<PathBuf, String> {
    let url = format!("https://github.com/{repo}.git");
    clone_repo_from_url(repo, parent, &url, branch, token, verbose)
}

fn clone_repo_from_url(
    repo: &str,
    parent: &Path,
    url: &str,
    branch: Option<&str>,
    token: Option<&str>,
    verbose: bool,
) -> Result<PathBuf, String> {
    let (org, name) = repo
        .split_once('/')
        .ok_or_else(|| format!("bad repo format: {:?}", repo))?;
    let repo_path = parent.join(org).join(name);

    if repo_path.exists() {
        validate_cached_clone(&repo_path, url)?;
    } else {
        if let Some(parent_dir) = repo_path.parent() {
            std::fs::create_dir_all(parent_dir).map_err(|e| format!("mkdir failed: {e}"))?;
        }
        std::fs::create_dir(&repo_path).map_err(|e| format!("mkdir failed: {e}"))?;
        run_git(
            Command::new("git").arg("init").arg("-q").arg(&repo_path),
            token,
            "git init",
        )?;
        run_git(
            Command::new("git")
                .arg("-C")
                .arg(&repo_path)
                .args(["remote", "add", "origin"])
                .arg(url),
            token,
            "git remote add",
        )?;
    }

    if verbose {
        eprintln!("Cloning https://github.com/{}.git ...", repo);
    }

    let mut fetch = Command::new("git");
    fetch
        .arg("-C")
        .arg(&repo_path)
        .args(["fetch", "--depth", "1", "origin"])
        .arg(branch.unwrap_or("HEAD"));
    run_git(&mut fetch, token, "git fetch")?;
    run_git(
        Command::new("git").arg("-C").arg(&repo_path).args([
            "checkout",
            "--detach",
            "--force",
            "FETCH_HEAD",
        ]),
        token,
        "git checkout",
    )?;

    if verbose {
        eprintln!("Cloned to {}", repo_path.display());
    }
    Ok(repo_path)
}

fn validate_cached_clone(repo_path: &Path, expected_url: &str) -> Result<(), String> {
    if !repo_path.join(".git").is_dir() {
        return Err(format!(
            "cached path is not a git repository: {}",
            repo_path.display()
        ));
    }
    let origin = git_stdout(repo_path, &["remote", "get-url", "origin"])?;
    if normalize_origin(&origin) != normalize_origin(expected_url) {
        return Err(format!(
            "cached clone origin mismatch: expected {}, found {}",
            expected_url, origin
        ));
    }
    let dirty = git_stdout(repo_path, &["status", "--porcelain"])?;
    if !dirty.is_empty() {
        return Err(format!(
            "cached clone has local changes: {}",
            repo_path.display()
        ));
    }
    Ok(())
}

fn normalize_origin(url: &str) -> String {
    let normalized = url
        .trim()
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_ascii_lowercase();
    normalized
        .strip_prefix("https://github.com/")
        .or_else(|| normalized.strip_prefix("http://github.com/"))
        .or_else(|| normalized.strip_prefix("ssh://git@github.com/"))
        .or_else(|| normalized.strip_prefix("git@github.com:"))
        .map(|path| format!("github.com/{path}"))
        .unwrap_or(normalized)
}

fn git_stdout(repo_path: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(args)
        .output()
        .map_err(|e| format!("git command failed: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run_git(cmd: &mut Command, token: Option<&str>, label: &str) -> Result<(), String> {
    let encoded_token = configure_git_auth(cmd, token);
    let output = cmd
        .output()
        .map_err(|e| format!("{label} failed to start: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = redact_credentials(
        String::from_utf8_lossy(&output.stderr).trim(),
        token,
        encoded_token.as_deref(),
    );
    Err(format!("{label} failed: {stderr}"))
}

fn configure_git_auth(cmd: &mut Command, token: Option<&str>) -> Option<String> {
    let token = token?;
    let encoded =
        base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}"));
    cmd.env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.extraHeader")
        .env(
            "GIT_CONFIG_VALUE_0",
            format!("Authorization: Basic {encoded}"),
        );
    Some(encoded)
}

fn redact_credentials(stderr: &str, token: Option<&str>, encoded: Option<&str>) -> String {
    let mut redacted = stderr.to_string();
    if let Some(token) = token {
        redacted = redacted.replace(token, "***");
    }
    if let Some(encoded) = encoded {
        redacted = redacted.replace(encoded, "***");
    }
    redacted
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn git(dir: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("git command");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn commit_file(repo: &Path, body: &str, message: &str) -> String {
        std::fs::write(repo.join("value.txt"), body).expect("write fixture");
        git(repo, &["add", "value.txt"]);
        git(repo, &["commit", "-qm", message]);
        git(repo, &["rev-parse", "HEAD"])
    }

    struct RemoteFixture {
        _temp: tempfile::TempDir,
        source: PathBuf,
        remote: PathBuf,
        cache: PathBuf,
        main_first: String,
    }

    fn remote_fixture() -> RemoteFixture {
        let temp = tempfile::tempdir().expect("tempdir");
        let source = temp.path().join("source");
        let remote = temp.path().join("remote.git");
        let cache = temp.path().join("cache");
        std::fs::create_dir(&source).expect("source dir");
        std::fs::create_dir(&cache).expect("cache dir");
        git(&source, &["init", "-q"]);
        git(&source, &["config", "user.email", "test@example.com"]);
        git(&source, &["config", "user.name", "Test"]);
        git(&source, &["checkout", "-qb", "main"]);
        let main_first = commit_file(&source, "main-one\n", "main one");
        git(&source, &["checkout", "-qb", "feature"]);
        commit_file(&source, "feature\n", "feature");
        git(&source, &["checkout", "main"]);
        let remote_text = remote.to_string_lossy();
        git(temp.path(), &["init", "--bare", "-q", &remote_text]);
        git(&source, &["remote", "add", "origin", &remote_text]);
        git(&source, &["push", "-q", "--all", "origin"]);
        git(&remote, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        RemoteFixture {
            _temp: temp,
            source,
            remote,
            cache,
            main_first,
        }
    }

    #[test]
    fn cached_clone_refreshes_and_switches_branches() {
        let fixture = remote_fixture();
        let remote = fixture.remote.to_string_lossy();
        let checkout = clone_repo_from_url(
            "org/repo",
            &fixture.cache,
            &remote,
            Some("main"),
            None,
            false,
        )
        .expect("first clone");
        assert_eq!(git(&checkout, &["rev-parse", "HEAD"]), fixture.main_first);

        let main_second = commit_file(&fixture.source, "main-two\n", "main two");
        git(&fixture.source, &["push", "-q", "origin", "main"]);
        clone_repo_from_url("org/repo", &fixture.cache, &remote, None, None, false)
            .expect("refresh default branch");
        assert_eq!(git(&checkout, &["rev-parse", "HEAD"]), main_second);

        clone_repo_from_url(
            "org/repo",
            &fixture.cache,
            &remote,
            Some("feature"),
            None,
            false,
        )
        .expect("switch branch");
        assert_eq!(
            std::fs::read_to_string(checkout.join("value.txt")).expect("read checkout"),
            "feature\n"
        );
    }

    #[test]
    fn cached_clone_rejects_non_repo_wrong_origin_and_dirty_state() {
        let fixture = remote_fixture();
        let remote = fixture.remote.to_string_lossy();
        let non_repo = fixture.cache.join("org/nonrepo");
        std::fs::create_dir_all(&non_repo).expect("non-repo dir");
        let error = clone_repo_from_url("org/nonrepo", &fixture.cache, &remote, None, None, false)
            .expect_err("non-repo must fail");
        assert!(error.contains("not a git repository"), "{error}");

        let checkout = clone_repo_from_url("org/repo", &fixture.cache, &remote, None, None, false)
            .expect("first clone");
        git(&checkout, &["remote", "set-url", "origin", "/wrong/origin"]);
        let error = clone_repo_from_url("org/repo", &fixture.cache, &remote, None, None, false)
            .expect_err("wrong origin must fail");
        assert!(error.contains("origin mismatch"), "{error}");

        git(&checkout, &["remote", "set-url", "origin", &remote]);
        std::fs::write(checkout.join("untracked.txt"), "dirty\n").expect("dirty checkout");
        let error = clone_repo_from_url("org/repo", &fixture.cache, &remote, None, None, false)
            .expect_err("dirty checkout must fail");
        assert!(error.contains("local changes"), "{error}");
    }

    #[test]
    fn credentials_stay_out_of_argv_and_are_fully_redacted() {
        let token = "raw-token:/?";
        let mut command = Command::new("git");
        command.args(["fetch", "https://github.com/org/repo.git"]);
        let encoded = configure_git_auth(&mut command, Some(token)).expect("encoded token");
        assert!(command
            .get_args()
            .all(|arg| !arg.to_string_lossy().contains(token)));
        let environment = command
            .get_envs()
            .filter_map(|(key, value)| value.map(|value| (key, value)))
            .map(|(key, value)| format!("{}={}", key.to_string_lossy(), value.to_string_lossy()))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!environment.contains(token));
        assert!(environment.contains(&encoded));
        let redacted = redact_credentials(
            &format!("raw={token} encoded={encoded}"),
            Some(token),
            Some(&encoded),
        );
        assert!(!redacted.contains(token));
        assert!(!redacted.contains(&encoded));
        assert_eq!(
            normalize_origin("git@github.com:Org/Repo.git"),
            "github.com/org/repo"
        );
        assert_eq!(OsStr::new("git"), command.get_program());
    }
}
