//! Git worktree-workspace для пишущих субагентов (stage 2 параллельных
//! субагентов).
//!
//! Lifecycle оркестрирует родительский workflow (см. решение в
//! docs/roadmap.md): он просит хост создать worktree, подменяет `task.cwd`
//! ребёнка на его путь и после `wait` просит cleanup. Здесь — только
//! механика поверх системного `git` (sync: вызывается с blocking-потока
//! workflow-хоста).

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};

use crate::contracts::WorkspaceInfo;

/// Каталог worktree-ов относительно repo root. Добавляется в
/// `.git/info/exclude`, а не в `.gitignore` репозитория.
const WORKTREES_DIR: &str = ".proteus/worktrees";
const EXCLUDE_LINE: &str = "/.proteus/";

/// Создаёт worktree `<repo_root>/.proteus/worktrees/<name>` на новой ветке
/// `proteus/<name>` от текущего HEAD. Не-git cwd, пустой репозиторий или
/// занятое имя — обычные ошибки (уходят модели как error ToolResult).
pub fn create_worktree(parent_cwd: &Path, name: &str) -> Result<WorkspaceInfo> {
    validate_name(name)?;
    let repo_root = PathBuf::from(
        run_git(parent_cwd, &["rev-parse", "--show-toplevel"])
            .context("subagent worktree requires a git repository")?,
    );
    let base_commit = run_git(&repo_root, &["rev-parse", "HEAD"])
        .context("subagent worktree requires at least one commit")?;

    let path = repo_root.join(WORKTREES_DIR).join(name);
    if path.exists() {
        bail!("worktree path already exists: {}", path.display());
    }
    let branch = format!("proteus/{name}");

    ensure_excluded(&repo_root)?;
    run_git(
        &repo_root,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            path.to_str()
                .with_context(|| format!("non-UTF-8 worktree path: {}", path.display()))?,
            "HEAD",
        ],
    )
    .context("failed to add git worktree")?;

    Ok(WorkspaceInfo::new(repo_root, path, branch, base_commit))
}

/// Убирает worktree, если ребёнок ничего не изменил: чистый `git status`
/// и HEAD == base_commit → `git worktree remove` + удаление ветки,
/// возвращает `true`. Изменения есть — worktree остаётся мержить родителю,
/// возвращает `false`. Worktree уже исчез с диска — прибирает bookkeeping
/// и возвращает `true`.
pub fn cleanup_worktree_if_unchanged(info: &WorkspaceInfo) -> Result<bool> {
    if !info.path.exists() {
        let _ = run_git(&info.repo_root, &["worktree", "prune"]);
        let _ = run_git(&info.repo_root, &["branch", "-D", &info.branch]);
        return Ok(true);
    }

    let status = run_git(&info.path, &["status", "--porcelain"])
        .context("failed to check worktree status")?;
    if !status.is_empty() {
        return Ok(false);
    }
    let head =
        run_git(&info.path, &["rev-parse", "HEAD"]).context("failed to resolve worktree HEAD")?;
    if head != info.base_commit {
        return Ok(false);
    }

    let path = info
        .path
        .to_str()
        .with_context(|| format!("non-UTF-8 worktree path: {}", info.path.display()))?;
    run_git(&info.repo_root, &["worktree", "remove", path])
        .context("failed to remove git worktree")?;
    run_git(&info.repo_root, &["branch", "-D", &info.branch])
        .context("failed to delete worktree branch")?;
    Ok(true)
}

/// Имя идёт в путь и в имя ветки — только безопасный алфавит.
fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("workspace name must not be empty");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        || name.starts_with('.')
    {
        bail!("workspace name must be [A-Za-z0-9._-]+ and not start with a dot: {name}");
    }
    Ok(())
}

/// Дописывает `/.proteus/` в `.git/info/exclude` основного checkout, чтобы
/// каталог worktree-ов не светился в `git status` (не трогая `.gitignore`).
fn ensure_excluded(repo_root: &Path) -> Result<()> {
    let git_dir = PathBuf::from(run_git(repo_root, &["rev-parse", "--git-common-dir"])?);
    let git_dir = if git_dir.is_absolute() {
        git_dir
    } else {
        repo_root.join(git_dir)
    };
    let info_dir = git_dir.join("info");
    let exclude = info_dir.join("exclude");
    let existing = fs::read_to_string(&exclude).unwrap_or_default();
    if existing.lines().any(|line| line.trim() == EXCLUDE_LINE) {
        return Ok(());
    }
    fs::create_dir_all(&info_dir)
        .with_context(|| format!("failed to create {}", info_dir.display()))?;
    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(EXCLUDE_LINE);
    updated.push('\n');
    fs::write(&exclude, updated).with_context(|| format!("failed to write {}", exclude.display()))
}

/// Запускает git и возвращает trimmed stdout; не-нулевой exit code —
/// ошибка с stderr в тексте.
fn run_git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        run_git(dir, args).expect("git command");
    }

    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(dir.path(), &["config", "user.email", "test@test"]);
        git(dir.path(), &["config", "user.name", "test"]);
        fs::write(dir.path().join("file.txt"), "base\n").expect("seed file");
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-q", "-m", "base"]);
        dir
    }

    #[test]
    fn create_and_cleanup_unchanged_worktree() {
        let repo = init_repo();
        let info = create_worktree(repo.path(), "task-1").expect("worktree");

        assert!(info.path.is_dir());
        assert_eq!(info.branch, "proteus/task-1");
        assert_eq!(
            run_git(&info.path, &["rev-parse", "HEAD"]).unwrap(),
            info.base_commit
        );
        // Основной checkout не замусорен: .proteus/ исключён.
        assert_eq!(
            run_git(repo.path(), &["status", "--porcelain"]).unwrap(),
            ""
        );

        assert!(cleanup_worktree_if_unchanged(&info).expect("cleanup"));
        assert!(!info.path.exists());
        assert!(
            run_git(repo.path(), &["branch", "--list", &info.branch])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn cleanup_keeps_worktree_with_uncommitted_changes() {
        let repo = init_repo();
        let info = create_worktree(repo.path(), "task-2").expect("worktree");
        fs::write(info.path.join("file.txt"), "changed\n").expect("edit");

        assert!(!cleanup_worktree_if_unchanged(&info).expect("cleanup"));
        assert!(info.path.is_dir());
    }

    #[test]
    fn cleanup_keeps_worktree_with_commits() {
        let repo = init_repo();
        let info = create_worktree(repo.path(), "task-3").expect("worktree");
        fs::write(info.path.join("new.txt"), "work\n").expect("edit");
        git(&info.path, &["add", "."]);
        git(&info.path, &["commit", "-q", "-m", "child work"]);

        assert!(!cleanup_worktree_if_unchanged(&info).expect("cleanup"));
        assert!(info.path.is_dir());
    }

    #[test]
    fn cleanup_of_manually_removed_worktree_prunes_bookkeeping() {
        let repo = init_repo();
        let info = create_worktree(repo.path(), "task-4").expect("worktree");
        fs::remove_dir_all(&info.path).expect("manual removal");

        assert!(cleanup_worktree_if_unchanged(&info).expect("cleanup"));
        assert!(
            run_git(repo.path(), &["branch", "--list", &info.branch])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn worktree_in_subdirectory_resolves_repo_root() {
        let repo = init_repo();
        let sub = repo.path().join("src");
        fs::create_dir(&sub).expect("subdir");
        let info = create_worktree(&sub, "task-5").expect("worktree");
        assert!(
            info.path
                .starts_with(repo.path().canonicalize().expect("canonical repo root"))
                || info.path.starts_with(repo.path())
        );
    }

    #[test]
    fn non_git_cwd_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = create_worktree(dir.path(), "task-6").unwrap_err();
        assert!(format!("{error:#}").contains("git repository"));
    }

    #[test]
    fn duplicate_and_invalid_names_are_rejected() {
        let repo = init_repo();
        create_worktree(repo.path(), "dup").expect("worktree");
        assert!(create_worktree(repo.path(), "dup").is_err());
        assert!(create_worktree(repo.path(), "").is_err());
        assert!(create_worktree(repo.path(), "../evil").is_err());
        assert!(create_worktree(repo.path(), ".hidden").is_err());
    }
}
