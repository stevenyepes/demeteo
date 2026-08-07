// Tests extracted from `crates/demeteo-core/src/shared/win/posix_shell.rs`
// (mirrored-tests convention). `super` = that module.
//
// These run on Linux, which is the point: no Windows cross-compiler exists on
// the development host, so a decision only a Windows build could exercise
// would be a decision nobody sees until CI. Paths are compared through `norm`
// so the same assertions hold when CI runs this file on windows-latest, where
// `Path::join` uses the other separator.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn norm(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn norm_all(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|p| norm(p)).collect()
}

/// A host that knows only what it was told. `bash_version` errors on any path
/// it has no answer for, so a candidate the resolver should never have probed
/// fails the test instead of quietly returning a default (AGENTS.md §7).
#[derive(Default)]
struct FakeHost {
    files: BTreeSet<String>,
    answers: BTreeMap<String, String>,
    asked: std::cell::RefCell<Vec<String>>,
}

impl FakeHost {
    fn with_bash(paths: &[&str], version: &str) -> Self {
        let mut host = FakeHost::default();
        for path in paths {
            host.files.insert((*path).to_string());
            host.answers
                .insert((*path).to_string(), version.to_string());
            let sh = path.replace("bash.exe", "sh.exe");
            host.files.insert(sh);
        }
        host
    }

    fn file(mut self, path: &str) -> Self {
        self.files.insert(path.to_string());
        self
    }

    fn without_file(mut self, path: &str) -> Self {
        self.files.remove(path);
        self
    }

    fn answering(mut self, path: &str, answer: &str) -> Self {
        self.files.insert(path.to_string());
        self.answers.insert(path.to_string(), answer.to_string());
        self
    }
}

impl ShellHost for FakeHost {
    fn is_file(&self, path: &Path) -> bool {
        self.files.contains(&norm(path))
    }

    fn bash_version(&self, bash: &Path) -> Result<String, String> {
        let key = norm(bash);
        self.asked.borrow_mut().push(key.clone());
        self.answers
            .get(&key)
            .cloned()
            .ok_or_else(|| format!("FakeHost was never told what {key} answers"))
    }
}

const BASH_5: &str = "5.2.37(1)-release\n";
const PF_GIT_BASH: &str = "C:/Program Files/Git/bin/bash.exe";
const PER_USER_BASH: &str = "C:/Users/dev/AppData/Local/Programs/Git/bin/bash.exe";

fn full_search() -> ShellSearch {
    ShellSearch {
        override_bash: None,
        registry_install_paths: vec![
            r"C:\Program Files\Git".to_string(),
            r"C:\Users\dev\AppData\Local\Programs\Git".to_string(),
        ],
        git_exec_path: Some("C:/Program Files/Git/mingw64/libexec/git-core".to_string()),
        git_exe: Some(r"C:\Users\dev\scoop\apps\git\current\cmd\git.exe".to_string()),
        program_files: Some(r"C:\Program Files".to_string()),
        program_files_x86: Some(r"C:\Program Files (x86)".to_string()),
        local_app_data: Some(r"C:\Users\dev\AppData\Local".to_string()),
    }
}

// ── source 1: the DEMETEO_BASH_PATH override ────────────────────────────────

#[test]
fn override_beats_every_other_source() {
    let mut search = full_search();
    search.override_bash = Some(r"D:\tools\git\bin\bash.exe".to_string());
    let host = FakeHost::with_bash(&["D:/tools/git/bin/bash.exe"], BASH_5);

    let shell = resolve(&search, &host).expect("override resolves");

    assert_eq!(norm(&shell.bash), "D:/tools/git/bin/bash.exe");
    assert_eq!(norm(&shell.root), "D:/tools/git");
    assert_eq!(norm(&shell.sh), "D:/tools/git/bin/sh.exe");
}

#[test]
fn a_broken_override_fails_instead_of_falling_back() {
    let mut search = full_search();
    search.override_bash = Some(r"D:\nowhere\bash.exe".to_string());
    let host = FakeHost::with_bash(&[PF_GIT_BASH], BASH_5);

    let err = resolve(&search, &host).expect_err("a wrong override must not be papered over");

    assert!(
        matches!(
            err,
            ShellMissing::OverrideUnusable {
                reason: Unusable::Absent,
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn override_of_an_msys2_layout_derives_the_root_above_usr() {
    let search = ShellSearch {
        override_bash: Some(r"C:\Program Files\Git\usr\bin\bash.exe".to_string()),
        ..ShellSearch::default()
    };
    let host = FakeHost::with_bash(&["C:/Program Files/Git/usr/bin/bash.exe"], BASH_5);

    let shell = resolve(&search, &host).expect("resolves");

    assert_eq!(norm(&shell.root), "C:/Program Files/Git");
}

// ── source 2: the GitForWindows registry key ────────────────────────────────

#[test]
fn registry_install_path_resolves() {
    let search = ShellSearch {
        registry_install_paths: vec![r"C:\Program Files\Git".to_string()],
        ..ShellSearch::default()
    };
    let host = FakeHost::with_bash(&[PF_GIT_BASH], BASH_5);

    let shell = resolve(&search, &host).expect("resolves");

    assert_eq!(norm(&shell.bash), PF_GIT_BASH);
}

#[test]
fn per_user_registry_install_path_resolves() {
    let search = ShellSearch {
        registry_install_paths: vec![r"C:\Users\dev\AppData\Local\Programs\Git".to_string()],
        ..ShellSearch::default()
    };
    let host = FakeHost::with_bash(&[PER_USER_BASH], BASH_5);

    let shell = resolve(&search, &host).expect("resolves");

    assert_eq!(norm(&shell.bash), PER_USER_BASH);
    assert_eq!(norm(&shell.root), "C:/Users/dev/AppData/Local/Programs/Git");
}

// ── source 3: derived from git ──────────────────────────────────────────────

#[test]
fn exec_path_forward_slashes_pop_three_components() {
    let root = root_from_exec_path("C:/Program Files/Git/mingw64/libexec/git-core")
        .expect("three components above git-core");

    assert_eq!(norm(&root), "C:/Program Files/Git");
}

#[test]
fn exec_path_resolves_when_no_registry_key_exists() {
    let search = ShellSearch {
        git_exec_path: Some("C:/Program Files/Git/mingw64/libexec/git-core".to_string()),
        ..ShellSearch::default()
    };
    let host = FakeHost::with_bash(&[PF_GIT_BASH], BASH_5);

    let shell = resolve(&search, &host).expect("resolves");

    assert_eq!(norm(&shell.bash), PF_GIT_BASH);
}

#[test]
fn exec_path_too_shallow_yields_no_root() {
    assert_eq!(root_from_exec_path("git-core"), None);
    assert_eq!(root_from_exec_path("libexec/git-core"), None);
}

#[test]
fn git_exe_in_cmd_yields_the_directory_above_it() {
    let roots = roots_from_git_exe(r"C:\Users\dev\scoop\apps\git\current\cmd\git.exe");

    assert_eq!(
        norm_all(&roots),
        vec!["C:/Users/dev/scoop/apps/git/current"]
    );
}

#[test]
fn git_exe_inside_the_msys_tree_yields_the_real_root_first() {
    let roots = roots_from_git_exe(r"C:\PortableGit\mingw64\bin\git.exe");

    assert_eq!(
        norm_all(&roots),
        vec!["C:/PortableGit", "C:/PortableGit/mingw64"]
    );
}

#[test]
fn git_exe_somewhere_unrecognised_yields_nothing() {
    assert!(roots_from_git_exe(r"C:\tools\git.exe").is_empty());
}

#[test]
fn git_exe_on_path_resolves_a_scoop_install() {
    let search = ShellSearch {
        git_exe: Some(r"C:\Users\dev\scoop\apps\git\current\cmd\git.exe".to_string()),
        ..ShellSearch::default()
    };
    let host = FakeHost::with_bash(
        &["C:/Users/dev/scoop/apps/git/current/bin/bash.exe"],
        BASH_5,
    );

    let shell = resolve(&search, &host).expect("resolves");

    assert_eq!(
        norm(&shell.bash),
        "C:/Users/dev/scoop/apps/git/current/bin/bash.exe"
    );
}

// ── source 4: the well-known directories ────────────────────────────────────

#[test]
fn program_files_is_probed_when_nothing_authoritative_points_anywhere() {
    let search = ShellSearch {
        program_files: Some(r"C:\Program Files".to_string()),
        program_files_x86: Some(r"C:\Program Files (x86)".to_string()),
        local_app_data: Some(r"C:\Users\dev\AppData\Local".to_string()),
        ..ShellSearch::default()
    };
    let host = FakeHost::with_bash(&["C:/Program Files (x86)/Git/bin/bash.exe"], BASH_5);

    let shell = resolve(&search, &host).expect("resolves");

    assert_eq!(norm(&shell.bash), "C:/Program Files (x86)/Git/bin/bash.exe");
}

#[test]
fn local_app_data_probe_includes_the_programs_segment() {
    let search = ShellSearch {
        local_app_data: Some(r"C:\Users\dev\AppData\Local".to_string()),
        ..ShellSearch::default()
    };

    let bashes = norm_all(
        &candidates(&search)
            .iter()
            .map(|c| c.bash.clone())
            .collect::<Vec<_>>(),
    );

    assert_eq!(
        bashes,
        vec![
            PER_USER_BASH,
            "C:/Users/dev/AppData/Local/Programs/Git/usr/bin/bash.exe"
        ]
    );
}

// ── ordering ────────────────────────────────────────────────────────────────

#[test]
fn candidate_order_is_authoritative_roots_then_guesses_and_bin_before_usr_bin() {
    let bashes = norm_all(
        &candidates(&full_search())
            .iter()
            .map(|c| c.bash.clone())
            .collect::<Vec<_>>(),
    );

    assert_eq!(
        bashes,
        vec![
            "C:/Program Files/Git/bin/bash.exe",
            "C:/Program Files/Git/usr/bin/bash.exe",
            "C:/Users/dev/AppData/Local/Programs/Git/bin/bash.exe",
            "C:/Users/dev/AppData/Local/Programs/Git/usr/bin/bash.exe",
            "C:/Users/dev/scoop/apps/git/current/bin/bash.exe",
            "C:/Users/dev/scoop/apps/git/current/usr/bin/bash.exe",
            "C:/Program Files (x86)/Git/bin/bash.exe",
            "C:/Program Files (x86)/Git/usr/bin/bash.exe",
        ]
    );
}

#[test]
fn bin_bash_wins_over_usr_bin_bash_in_the_same_install() {
    let search = ShellSearch {
        registry_install_paths: vec![r"C:\Program Files\Git".to_string()],
        ..ShellSearch::default()
    };
    let host = FakeHost::with_bash(
        &[PF_GIT_BASH, "C:/Program Files/Git/usr/bin/bash.exe"],
        BASH_5,
    );

    let shell = resolve(&search, &host).expect("resolves");

    assert_eq!(norm(&shell.bash), PF_GIT_BASH);
}

#[test]
fn usr_bin_bash_is_used_when_bin_has_none() {
    let search = ShellSearch {
        registry_install_paths: vec![r"C:\Program Files\Git".to_string()],
        ..ShellSearch::default()
    };
    let host = FakeHost::with_bash(&["C:/Program Files/Git/usr/bin/bash.exe"], BASH_5);

    let shell = resolve(&search, &host).expect("resolves");

    assert_eq!(norm(&shell.bash), "C:/Program Files/Git/usr/bin/bash.exe");
}

#[test]
fn no_candidate_is_ever_a_bare_or_system_bash() {
    for candidate in candidates(&full_search()) {
        let bash = norm(&candidate.bash);
        assert!(
            bash.contains('/'),
            "{bash} is a bare name a PATH search would resolve"
        );
        assert_eq!(rejection(&candidate.bash), None, "{bash}");
        assert!(!bash.ends_with("git-bash.exe"), "{bash}");
    }
}

// ── the two rejections ──────────────────────────────────────────────────────

#[test]
fn wsl_launcher_is_rejected() {
    let search = ShellSearch {
        override_bash: Some(r"C:\Windows\System32\bash.exe".to_string()),
        ..ShellSearch::default()
    };
    let host = FakeHost::with_bash(&["C:/Windows/System32/bash.exe"], BASH_5);

    let err = resolve(&search, &host).expect_err("WSL bash must never be accepted");

    assert!(
        matches!(
            err,
            ShellMissing::OverrideUnusable {
                reason: Unusable::WslLauncher,
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn every_system_directory_spelling_is_rejected() {
    for dir in ["System32", "SysWOW64", "Sysnative", "system32"] {
        let path = PathBuf::from(format!("C:/Windows/{dir}/bash.exe"));
        assert_eq!(rejection(&path), Some(Unusable::WslLauncher), "{dir}");
    }
}

#[test]
fn mintty_launcher_is_rejected() {
    let search = ShellSearch {
        override_bash: Some(r"C:\Program Files\Git\git-bash.exe".to_string()),
        ..ShellSearch::default()
    };
    let host = FakeHost::with_bash(&["C:/Program Files/Git/git-bash.exe"], BASH_5);

    let err = resolve(&search, &host).expect_err("git-bash.exe must never be accepted");

    assert!(
        matches!(
            err,
            ShellMissing::OverrideUnusable {
                reason: Unusable::MinttyLauncher,
                ..
            }
        ),
        "{err:?}"
    );
}

#[test]
fn a_rejected_candidate_is_never_probed() {
    let search = ShellSearch {
        override_bash: Some(r"C:\Windows\System32\bash.exe".to_string()),
        ..ShellSearch::default()
    };
    let host = FakeHost::with_bash(&["C:/Windows/System32/bash.exe"], BASH_5);

    let _ = resolve(&search, &host);

    assert!(host.asked.borrow().is_empty(), "{:?}", host.asked.borrow());
}

// ── BusyBox MinGit ──────────────────────────────────────────────────────────

#[test]
fn probe_verdict_reads_none_as_not_bash() {
    assert!(!probe_says_bash("none\n"));
    assert!(!probe_says_bash("none\r\n"));
    assert!(!probe_says_bash(""));
    assert!(!probe_says_bash("   \n"));
    assert!(probe_says_bash(BASH_5));
    assert!(probe_says_bash("4.4.23(1)-release\r\n"));
}

#[test]
fn busybox_mingit_is_rejected_and_names_itself() {
    let search = ShellSearch {
        registry_install_paths: vec![r"C:\Program Files\Git".to_string()],
        ..ShellSearch::default()
    };
    let host = FakeHost::default().answering(PF_GIT_BASH, "none\n");

    let err = resolve(&search, &host).expect_err("ash must not pass as bash");

    match &err {
        ShellMissing::NotBash { bash, answer } => {
            assert_eq!(norm(bash), PF_GIT_BASH);
            assert_eq!(answer, "none");
        }
        other => panic!("{other:?}"),
    }
    assert!(err.is_mingit());
}

#[test]
fn a_busybox_install_does_not_mask_a_real_one_further_down() {
    let search = ShellSearch {
        registry_install_paths: vec![
            r"C:\Program Files\Git".to_string(),
            r"C:\Users\dev\AppData\Local\Programs\Git".to_string(),
        ],
        ..ShellSearch::default()
    };
    let host = FakeHost::with_bash(&[PER_USER_BASH], BASH_5).answering(PF_GIT_BASH, "none\n");

    let shell = resolve(&search, &host).expect("the real install is still found");

    assert_eq!(norm(&shell.bash), PER_USER_BASH);
}

// ── nothing usable ──────────────────────────────────────────────────────────

#[test]
fn git_without_bash_is_the_mingit_signature() {
    let search = ShellSearch {
        registry_install_paths: vec![r"C:\Program Files\Git".to_string()],
        git_exec_path: Some("C:/Program Files/Git/mingw64/libexec/git-core".to_string()),
        ..ShellSearch::default()
    };
    let host = FakeHost::default();

    let err = resolve(&search, &host).expect_err("no bash anywhere");

    match &err {
        ShellMissing::GitWithoutBash {
            git_roots,
            searched,
        } => {
            assert_eq!(norm_all(git_roots), vec!["C:/Program Files/Git"]);
            assert_eq!(norm_all(searched).len(), 2);
        }
        other => panic!("{other:?}"),
    }
    assert!(err.is_mingit());
}

#[test]
fn no_git_at_all_is_not_a_mingit_diagnosis() {
    let search = ShellSearch {
        program_files: Some(r"C:\Program Files".to_string()),
        ..ShellSearch::default()
    };
    let host = FakeHost::default();

    let err = resolve(&search, &host).expect_err("nothing to find");

    assert!(
        matches!(err, ShellMissing::NoGitForWindows { .. }),
        "{err:?}"
    );
    assert!(!err.is_mingit());
}

#[test]
fn an_empty_search_finds_nothing_rather_than_guessing() {
    let err =
        resolve(&ShellSearch::default(), &FakeHost::default()).expect_err("no sources at all");

    assert!(
        matches!(err, ShellMissing::NoGitForWindows { .. }),
        "{err:?}"
    );
}

#[test]
fn a_shell_that_cannot_be_run_is_reported_as_unrunnable() {
    let search = ShellSearch {
        override_bash: Some(r"C:\Program Files\Git\bin\bash.exe".to_string()),
        ..ShellSearch::default()
    };
    let host = FakeHost::default().file(PF_GIT_BASH);

    let err = resolve(&search, &host).expect_err("the probe has no answer for it");

    assert!(
        matches!(
            err,
            ShellMissing::OverrideUnusable {
                reason: Unusable::Unrunnable(_),
                ..
            }
        ),
        "{err:?}"
    );
}

// ── sh ──────────────────────────────────────────────────────────────────────

#[test]
fn sh_is_the_sibling_of_the_chosen_bash() {
    let search = ShellSearch {
        registry_install_paths: vec![r"C:\Program Files\Git".to_string()],
        ..ShellSearch::default()
    };
    let host = FakeHost::with_bash(&[PF_GIT_BASH], BASH_5);

    let shell = resolve(&search, &host).expect("resolves");

    assert_eq!(norm(&shell.sh), "C:/Program Files/Git/bin/sh.exe");
}

#[test]
fn sh_falls_back_to_bash_when_the_install_ships_none() {
    let search = ShellSearch {
        registry_install_paths: vec![r"C:\Program Files\Git".to_string()],
        ..ShellSearch::default()
    };
    let host =
        FakeHost::with_bash(&[PF_GIT_BASH], BASH_5).without_file("C:/Program Files/Git/bin/sh.exe");

    let shell = resolve(&search, &host).expect("resolves");

    assert_eq!(norm(&shell.sh), PF_GIT_BASH);
}
