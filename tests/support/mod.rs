pub mod helpers {
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use tempfile::TempDir;

    pub fn snp_cmd() -> Command {
        Command::new(env!("CARGO_BIN_EXE_snp"))
    }

    pub fn setup_test_env() -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let config_dir = tmp.path().join(".config").join("snp");
        fs::create_dir_all(&config_dir).unwrap();
        (tmp, config_dir)
    }

    pub fn snp_in(config_dir: &Path) -> Command {
        let mut cmd = snp_cmd();
        cmd.env("XDG_CONFIG_HOME", config_dir.parent().unwrap());
        cmd
    }

    #[allow(dead_code)]
    pub fn output_with_stdin(mut cmd: Command, input: &[u8]) -> std::process::Output {
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        child.wait_with_output().unwrap()
    }

    #[allow(dead_code)]
    pub fn create_sort_test_library(config_dir: &Path, lib_name: &str) {
        let mut cmd = snp_in(config_dir);
        cmd.args(["library", "create", lib_name]);
        cmd.output().unwrap();

        let libraries_dir = config_dir.join("libraries");
        fs::create_dir_all(&libraries_dir).unwrap();
        let lib_path = libraries_dir.join(format!("{lib_name}.toml"));
        fs::write(
            &lib_path,
            r#"
[[Snippets]]
id = "test-1"
description = "zebra list"
command = "ls -la"
tags = ["files"]
output = ""
folders = []
favorite = false
created_at = 100
updated_at = 100

[[Snippets]]
id = "test-2"
description = "alpha deploy"
command = "deploy.sh"
tags = ["deploy"]
output = ""
folders = []
favorite = true
created_at = 300
updated_at = 300

[[Snippets]]
id = "test-3"
description = "middle status"
command = "git status"
tags = ["git"]
output = ""
folders = []
favorite = false
created_at = 200
updated_at = 200
"#,
        )
        .unwrap();

        let mut cmd = snp_in(config_dir);
        cmd.args(["library", "set-primary", lib_name]);
        cmd.output().unwrap();
    }

    #[allow(dead_code)]
    pub fn golden_corpus() -> Vec<(&'static str, &'static str)> {
        vec![
            ("ascii_simple", "echo hello world"),
            ("leading_hyphen", "-n echo 'leading flag'"),
            ("quotes", "echo \"double\" 'single'"),
            ("backslashes", "echo C:\\Users\\test\\file.txt"),
            ("shell_ops", "echo foo | grep bar > out.txt; echo baz &"),
            ("substitution", "echo $(date) `whoami`"),
            ("unicode", "echo '日本語 test café'"),
            ("leading_spaces", "  echo indented"),
            (
                "multiline_script",
                "if true; then\n  echo yes\nelse\n  echo no\nfi\n",
            ),
            ("blank_lines", "echo before\n\necho after\n"),
            ("no_trailing_newline", "echo no_newline"),
            ("one_trailing_newline", "echo with_newline\n"),
            ("multi_trailing_newlines", "echo multi\n\n\n"),
            ("variables", "ssh <user>@<host> -p <port=22>"),
            ("escaped_angle_brackets", "echo \\<literal\\> text"),
            ("tab_internal", "echo\there"),
            ("tab_makefile", "if true; then\n\techo yes\nfi\n"),
            ("trailing_space", "echo hello "),
            ("trailing_spaces_multi", "echo hello   "),
            ("crlf", "echo foo\r\necho bar\r\n"),
            ("mixed_newlines", "echo foo\r\necho bar\n"),
            ("tab_backslash", "echo \\path\\there"),
            ("tab_quotes", "echo \"hello\tworld\""),
            ("tab_trailing", "echo hello\t  \r\n"),
        ]
    }
}
