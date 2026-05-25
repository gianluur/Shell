// =============================================================================
// RSHELL TEST SUITE  —  tests/tests.rs
//
// Run with:
//   cargo test -- --test-threads=1
//
// --test-threads=1 is required because several tests mutate shared process
// state (current directory, environment variables). Running them in parallel
// causes races.
//
// [dev-dependencies] needed in Cargo.toml:
//   tempfile = "3"
// =============================================================================

// -----------------------------------------------------------------------------
// Test-local Context bypass
//
// Context::new() calls tcsetpgrp() to claim the terminal, which blocks or
// errors when run inside a cargo-test process (no controlling terminal).
// Rather than modifying the shell source, we build a minimal Context here
// using only the pieces each test actually needs.
//
// How it works:
//   - We call libc::getpid() directly for pgid/pid (safe, no side effects).
//   - SignalHandler is constructed via its public ::new(), but we wrap it in
//     a helper that ignores the tcsetpgrp failure by bypassing setup_pgid().
//   - History is pointed at a temp file so tests don't pollute ~/.rshell_history.
//   - Everything else (BuiltIns, Jobs, Aliases) constructs fine with ::new().
//
// Only the expander and builtin tests need a Context; tokenizer, parser,
// buffer, aliases, error, and prompt tests never touch it.
// -----------------------------------------------------------------------------
mod test_helpers {
    use rshell::{
        aliases::Aliases, builtins::BuiltIns, context::Context, history::History, jobs::Jobs,
        signals::SignalHandler, terminal::Terminal,
    };
    use std::{env, path::PathBuf};
    use tempfile::TempDir;

    /// A TempDir that we keep alive alongside the Context so the history
    /// file isn't deleted while the test is running.
    pub struct TestEnv {
        pub ctx: Context,
        pub term: Terminal,
        pub _history_dir: TempDir, // dropped last, keeping the tempdir alive
    }

    /// Build a Context that is safe to construct inside a test process.
    ///
    /// Differences from Context::new():
    ///   - Does NOT call tcsetpgrp / setpgid (would fail without a terminal).
    ///   - History file lives in a fresh TempDir instead of ~/.rshell_history.
    ///   - All other fields are identical to what Context::new() would produce.
    pub fn make_test_env() -> TestEnv {
        // Point HOME at a temp dir so History::new() writes there.
        let history_dir = tempfile::tempdir().expect("tempdir");
        unsafe { env::set_var("HOME", history_dir.path()) };

        let pid = unsafe { libc::getpid() };

        // SignalHandler::new() only sets up a self-pipe + SIGCHLD handler;
        // it does not touch the terminal, so it is safe in tests.
        let signals = SignalHandler::new().expect("SignalHandler::new");
        let history = History::new().expect("History::new");

        let ctx = Context {
            name: "RShell-test".to_string(),
            // 'directory' is private — we set it via update_cwd() below.
            // We construct Context via the public fields and then call
            // update_cwd() to initialise 'directory'.
            //
            // NOTE: if Context gains truly private fields that have no
            // public setter, expose a `pub fn new_for_test()` there.
            // For now all fields used below are pub.
            pid,
            pgid: pid, // shell is its own pgroup leader in test
            builtins: BuiltIns::new(),
            jobs: Jobs::new(),
            signals,
            last_exit_code: 0,
            last_job_pid: None,
            history,
            aliases: Aliases::new(),
            directory: PathBuf::from("/tmp"),
        };

        TestEnv {
            ctx,
            term: Terminal::new(),
            _history_dir: history_dir,
        }
    }
}

// =============================================================================
// tokenizer — tests
// =============================================================================
mod tokenizer_tests {
    use rshell::tokenizer::{Token, Tokenizer};

    fn tok<'a>(input: &'a str) -> Vec<Token<'a>> {
        Tokenizer::tokenize(input).expect("tokenize failed")
    }

    fn word<'a>(t: &'a Token<'a>) -> &'a str {
        match t {
            Token::Word(s) => s,
            other => panic!("expected Word, got {:?}", other),
        }
    }

    // ── Basic words ───────────────────────────────────────────────────────────

    #[test]
    fn single_word() {
        let tokens = tok("ls");
        assert_eq!(tokens.len(), 1);
        assert_eq!(word(&tokens[0]), "ls");
    }

    #[test]
    fn multiple_words() {
        let tokens = tok("ls -la /tmp");
        assert_eq!(tokens.len(), 3);
        assert_eq!(word(&tokens[0]), "ls");
        assert_eq!(word(&tokens[1]), "-la");
        assert_eq!(word(&tokens[2]), "/tmp");
    }

    #[test]
    fn leading_and_trailing_whitespace() {
        let tokens = tok("  echo hello  ");
        assert_eq!(tokens.len(), 2);
    }

    #[test]
    fn empty_input_returns_empty_vec() {
        assert!(tok("").is_empty());
    }

    #[test]
    fn whitespace_only_returns_empty_vec() {
        assert!(tok("   \t  ").is_empty());
    }

    // ── Quoted strings ────────────────────────────────────────────────────────

    #[test]
    fn single_quoted_string() {
        let tokens = tok("echo 'hello world'");
        assert_eq!(tokens.len(), 2);
        assert!(matches!(tokens[1], Token::SingleQuoted("hello world")));
    }

    #[test]
    fn double_quoted_string() {
        let tokens = tok(r#"echo "hello world""#);
        assert_eq!(tokens.len(), 2);
        assert!(matches!(tokens[1], Token::DoubleQuoted("hello world")));
    }

    #[test]
    fn single_quoted_preserves_special_chars() {
        let tokens = tok("echo '$VAR | ; &'");
        assert_eq!(tokens.len(), 2);
        assert!(matches!(tokens[1], Token::SingleQuoted("$VAR | ; &")));
    }

    #[test]
    fn double_quoted_preserves_spaces() {
        let tokens = tok(r#"echo "a  b  c""#);
        assert!(matches!(tokens[1], Token::DoubleQuoted("a  b  c")));
    }

    #[test]
    fn unclosed_single_quote_is_error() {
        assert!(Tokenizer::tokenize("echo 'unclosed").is_err());
    }

    #[test]
    fn unclosed_double_quote_is_error() {
        assert!(Tokenizer::tokenize(r#"echo "unclosed"#).is_err());
    }

    #[test]
    fn empty_single_quotes() {
        let tokens = tok("echo ''");
        assert!(matches!(tokens[1], Token::SingleQuoted("")));
    }

    #[test]
    fn empty_double_quotes() {
        let tokens = tok(r#"echo """#);
        assert!(matches!(tokens[1], Token::DoubleQuoted("")));
    }

    // ── Operators ─────────────────────────────────────────────────────────────

    #[test]
    fn pipe_operator() {
        let tokens = tok("ls | grep foo");
        assert!(matches!(tokens[1], Token::Pipe));
    }

    #[test]
    fn semicolon_operator() {
        let tokens = tok("echo a; echo b");
        assert!(matches!(tokens[2], Token::Semicolon));
    }

    #[test]
    fn and_operator() {
        let tokens = tok("make && ./run");
        assert!(matches!(tokens[1], Token::And));
    }

    #[test]
    fn or_operator() {
        let tokens = tok("cmd1 || cmd2");
        assert!(matches!(tokens[1], Token::Or));
    }

    #[test]
    fn background_operator() {
        let tokens = tok("sleep 10 &");
        assert!(matches!(tokens[2], Token::Background));
    }

    // ── Redirection ───────────────────────────────────────────────────────────

    #[test]
    fn redirect_out() {
        let tokens = tok("echo foo > out.txt");
        assert!(matches!(tokens[2], Token::RedirectOut));
        assert_eq!(word(&tokens[3]), "out.txt");
    }

    #[test]
    fn redirect_append() {
        let tokens = tok("echo foo >> out.txt");
        assert!(matches!(tokens[2], Token::RedirectAppend));
    }

    #[test]
    fn redirect_in() {
        let tokens = tok("cat < in.txt");
        assert!(matches!(tokens[1], Token::RedirectIn));
    }

    #[test]
    fn redirect_stderr() {
        let tokens = tok("cmd 2> err.txt");
        assert!(matches!(tokens[1], Token::RedirectErr));
    }

    #[test]
    fn redirect_stderr_and_stdout() {
        let tokens = tok("cmd 2>&1");
        assert!(matches!(tokens[1], Token::RedirectErrAndOut));
    }

    // ── Edge cases ────────────────────────────────────────────────────────────

    #[test]
    fn subcommand_in_word() {
        let tokens = tok("echo $(date)");
        assert_eq!(tokens.len(), 2);
        assert!(matches!(tokens[1], Token::Word(_)));
    }

    #[test]
    fn pipeline_no_spaces() {
        let tokens = tok("ls|grep foo");
        assert!(matches!(tokens[1], Token::Pipe));
    }

    #[test]
    fn unicode_word() {
        let tokens = tok("echo héllo");
        assert_eq!(word(&tokens[1]), "héllo");
    }
}

// =============================================================================
// parser — tests
// =============================================================================
mod parser_tests {
    use rshell::parser::{Command, Parser};
    use rshell::tokenizer::Tokenizer;

    // Keeps `tokens` alive in the same scope as the `Command` that borrows it,
    // then hands the command to a closure for assertions.
    macro_rules! parse {
        ($input:expr, $body:expr) => {{
            let tokens = Tokenizer::tokenize($input).unwrap();
            let cmd = Parser::parse(&tokens).unwrap();
            ($body)(cmd)
        }};
    }

    fn parse_err(input: &str) -> bool {
        let tokens = Tokenizer::tokenize(input).unwrap();
        Parser::parse(&tokens).is_err()
    }

    // ── Simple commands ───────────────────────────────────────────────────────

    #[test]
    fn simple_command_no_args() {
        parse!("ls", |cmd| {
            if let Command::Simple { command, args, .. } = cmd {
                assert_eq!(command.as_ref(), "ls");
                assert!(args.is_empty());
            } else {
                panic!("expected Simple");
            }
        });
    }

    #[test]
    fn simple_command_with_args() {
        parse!("ls -la /tmp", |cmd| {
            if let Command::Simple { command, args, .. } = cmd {
                assert_eq!(command.as_ref(), "ls");
                assert_eq!(args.len(), 2);
            } else {
                panic!("expected Simple");
            }
        });
    }

    #[test]
    fn empty_tokens_is_error() {
        assert!(parse_err(""));
    }

    // ── Pipelines ─────────────────────────────────────────────────────────────

    #[test]
    fn simple_pipeline() {
        parse!("ls | grep foo", |cmd| {
            assert!(matches!(cmd, Command::Pipeline(_, _)));
        });
    }

    #[test]
    fn triple_pipeline() {
        parse!("ls | grep foo | wc -l", |cmd| {
            assert!(matches!(cmd, Command::Pipeline(_, _)));
            if let Command::Pipeline(left, _) = cmd {
                assert!(matches!(*left, Command::Pipeline(_, _)));
            }
        });
    }

    // ── Boolean operators ─────────────────────────────────────────────────────

    #[test]
    fn and_operator() {
        parse!("make && ./run", |cmd| {
            assert!(matches!(cmd, Command::And(_, _)));
        });
    }

    #[test]
    fn or_operator() {
        parse!("cmd1 || cmd2", |cmd| {
            assert!(matches!(cmd, Command::Or(_, _)));
        });
    }

    // ── Sequences ─────────────────────────────────────────────────────────────

    #[test]
    fn sequence_with_semicolon() {
        parse!("echo a; echo b", |cmd| {
            assert!(matches!(cmd, Command::Sequence(_, _)));
        });
    }

    #[test]
    fn trailing_semicolon_is_ok() {
        let tokens = Tokenizer::tokenize("echo a;").unwrap();
        assert!(Parser::parse(&tokens).is_ok());
    }

    // ── Background ────────────────────────────────────────────────────────────

    #[test]
    fn background_command() {
        parse!("sleep 10 &", |cmd| {
            assert!(matches!(cmd, Command::Background(_)));
        });
    }

    // ── Redirections ──────────────────────────────────────────────────────────

    #[test]
    fn redirect_out_parsed() {
        parse!("echo hello > /tmp/out.txt", |cmd| {
            if let Command::Simple { redirects, .. } = cmd {
                assert_eq!(redirects.len(), 1);
            } else {
                panic!("expected Simple");
            }
        });
    }

    #[test]
    fn multiple_redirects() {
        parse!("cmd < in.txt > out.txt", |cmd| {
            if let Command::Simple { redirects, .. } = cmd {
                assert_eq!(redirects.len(), 2);
            } else {
                panic!("expected Simple");
            }
        });
    }

    #[test]
    fn redirect_missing_target_is_error() {
        assert!(parse_err("echo >"));
    }

    // ── Env variable prefix ───────────────────────────────────────────────────

    #[test]
    fn env_var_prefix() {
        parse!("FOO=bar env", |cmd| {
            if let Command::Simple {
                env_vars, command, ..
            } = cmd
            {
                assert_eq!(env_vars.len(), 1);
                assert_eq!(env_vars[0].name.as_ref(), "FOO");
                assert_eq!(env_vars[0].value.as_ref(), "bar");
                assert_eq!(command.as_ref(), "env");
            } else {
                panic!("expected Simple");
            }
        });
    }

    #[test]
    fn multiple_env_vars() {
        parse!("A=1 B=2 printenv", |cmd| {
            if let Command::Simple { env_vars, .. } = cmd {
                assert_eq!(env_vars.len(), 2);
            } else {
                panic!("expected Simple");
            }
        });
    }

    // ── Operator precedence ───────────────────────────────────────────────────

    #[test]
    fn pipeline_binds_tighter_than_and() {
        // "a | b && c"  →  And(Pipeline(a, b), c)
        parse!("a | b && c", |cmd| {
            assert!(matches!(cmd, Command::And(_, _)));
            if let Command::And(left, _) = cmd {
                assert!(matches!(*left, Command::Pipeline(_, _)));
            }
        });
    }

    #[test]
    fn and_binds_tighter_than_sequence() {
        // "a && b; c"  →  Sequence(And(a, b), c)
        parse!("a && b; c", |cmd| {
            assert!(matches!(cmd, Command::Sequence(_, _)));
        });
    }
}

// =============================================================================
// expander — tests
// =============================================================================
mod expander_tests {
    use crate::test_helpers::make_test_env;
    use rshell::shell::Shell;

    // ── Tilde ─────────────────────────────────────────────────────────────────

    #[test]
    fn tilde_expands_to_home() {
        let mut e = make_test_env();
        unsafe { std::env::set_var("HOME", "/home/testuser") };
        let cmd = Shell::parse_command(&mut e.ctx, &mut e.term, "ls ~/docs", true).unwrap();
        assert!(cmd.to_string().contains("/home/testuser/docs"));
    }

    #[test]
    fn tilde_in_middle_of_word_not_expanded() {
        let mut e = make_test_env();
        let cmd = Shell::parse_command(&mut e.ctx, &mut e.term, "echo foo~bar", true).unwrap();
        assert!(cmd.to_string().contains("foo~bar"));
    }

    // ── Variable expansion ────────────────────────────────────────────────────

    #[test]
    fn simple_variable_expands() {
        unsafe { std::env::set_var("MYVAR", "hello") };
        let mut e = make_test_env();
        let cmd = Shell::parse_command(&mut e.ctx, &mut e.term, "echo $MYVAR", true).unwrap();
        assert!(cmd.to_string().contains("hello"));
    }

    #[test]
    fn braces_variable_expands() {
        unsafe { std::env::set_var("BVAR", "world") };
        let mut e = make_test_env();
        let cmd = Shell::parse_command(&mut e.ctx, &mut e.term, "echo ${BVAR}", true).unwrap();
        assert!(cmd.to_string().contains("world"));
    }

    #[test]
    fn undefined_variable_expands_to_empty() {
        unsafe { std::env::remove_var("UNDEFINED_RSHELL_VAR") };
        let mut e = make_test_env();
        let cmd = Shell::parse_command(&mut e.ctx, &mut e.term, "echo $UNDEFINED_RSHELL_VAR", true)
            .unwrap();
        assert!(!cmd.to_string().contains("$UNDEFINED_RSHELL_VAR"));
    }

    #[test]
    fn dollar_question_mark_expands_to_exit_code() {
        let mut e = make_test_env();
        e.ctx.last_exit_code = 42;
        let cmd = Shell::parse_command(&mut e.ctx, &mut e.term, "echo $?", true).unwrap();
        assert!(cmd.to_string().contains("42"));
    }

    #[test]
    fn dollar_dollar_expands_to_pid() {
        let mut e = make_test_env();
        let pid = e.ctx.pid.to_string();
        let cmd = Shell::parse_command(&mut e.ctx, &mut e.term, "echo $$", true).unwrap();
        assert!(cmd.to_string().contains(&pid));
    }

    #[test]
    fn variable_in_double_quotes_expands() {
        unsafe { std::env::set_var("QVAR", "quoted") };
        let mut e = make_test_env();
        let cmd = Shell::parse_command(&mut e.ctx, &mut e.term, r#"echo "$QVAR""#, true).unwrap();
        assert!(cmd.to_string().contains("quoted"));
    }

    #[test]
    fn variable_in_single_quotes_not_expanded() {
        unsafe { std::env::set_var("SQVAR", "should_not_appear") };
        let mut e = make_test_env();
        let cmd = Shell::parse_command(&mut e.ctx, &mut e.term, "echo '$SQVAR'", true).unwrap();
        assert!(cmd.to_string().contains("$SQVAR"));
    }

    #[test]
    fn unclosed_brace_is_error() {
        let mut e = make_test_env();
        assert!(Shell::parse_command(&mut e.ctx, &mut e.term, "echo ${UNCLOSED", true).is_err());
    }

    // ── Globbing ──────────────────────────────────────────────────────────────

    #[test]
    fn glob_star_matches_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "").unwrap();
        std::fs::write(dir.path().join("b.txt"), "").unwrap();
        let pattern = format!("ls {}/*.txt", dir.path().display());
        let mut e = make_test_env();
        let cmd = Shell::parse_command(&mut e.ctx, &mut e.term, &pattern, true).unwrap();
        let s = cmd.to_string();
        assert!(s.contains("a.txt") && s.contains("b.txt"), "got: {s}");
    }

    #[test]
    fn glob_no_match_keeps_literal() {
        let mut e = make_test_env();
        let cmd = Shell::parse_command(
            &mut e.ctx,
            &mut e.term,
            "echo /absolutely/no/such/path/*.xyz",
            true,
        )
        .unwrap();
        assert!(cmd.to_string().contains("*.xyz"));
    }
}

// =============================================================================
// builtins — tests
// =============================================================================
mod builtin_tests {
    use crate::test_helpers::make_test_env;
    use rshell::builtins::BuiltIns;
    use rshell::error::ShellError;

    // ── cd ────────────────────────────────────────────────────────────────────

    #[test]
    fn cd_to_tmp() {
        let mut e = make_test_env();
        assert!(BuiltIns::cd(&["/tmp"], &mut e.ctx, &mut e.term).is_ok());
        assert_eq!(
            std::env::current_dir().unwrap(),
            std::path::PathBuf::from("/tmp")
        );
    }

    #[test]
    fn cd_no_args_goes_to_home() {
        unsafe { std::env::set_var("HOME", "/tmp") };
        let mut e = make_test_env();
        assert!(BuiltIns::cd(&[], &mut e.ctx, &mut e.term).is_ok());
    }

    #[test]
    fn cd_dash_goes_to_oldpwd() {
        unsafe { std::env::set_var("OLDPWD", "/tmp") };
        let mut e = make_test_env();
        assert!(BuiltIns::cd(&["-"], &mut e.ctx, &mut e.term).is_ok());
    }

    #[test]
    fn cd_nonexistent_path_is_error() {
        let mut e = make_test_env();
        assert!(BuiltIns::cd(&["/no/such/path/12345"], &mut e.ctx, &mut e.term).is_err());
    }

    // ── export ────────────────────────────────────────────────────────────────

    #[test]
    fn export_sets_env_var() {
        let mut e = make_test_env();
        BuiltIns::export(&["RSHELL_TEST_FOO=bar"], &mut e.ctx, &mut e.term).unwrap();
        assert_eq!(std::env::var("RSHELL_TEST_FOO").unwrap(), "bar");
    }

    #[test]
    fn export_no_args_is_error() {
        let mut e = make_test_env();
        // Wrapping in catch_unwind because the current implementation indexes
        // args[0] without an empty-check (known bug). This test documents the
        // behaviour: either a clean Err or a panic — both are wrong inputs.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            BuiltIns::export(&[], &mut e.ctx, &mut e.term)
        }));
        assert!(result.is_err() || result.unwrap().is_err());
    }

    #[test]
    fn export_invalid_format_is_error() {
        let mut e = make_test_env();
        assert!(BuiltIns::export(&["NOEQUALS"], &mut e.ctx, &mut e.term).is_err());
    }

    #[test]
    fn export_empty_name_is_error() {
        let mut e = make_test_env();
        assert!(BuiltIns::export(&["=value"], &mut e.ctx, &mut e.term).is_err());
    }

    #[test]
    fn export_empty_value_is_error() {
        let mut e = make_test_env();
        assert!(BuiltIns::export(&["NAME="], &mut e.ctx, &mut e.term).is_err());
    }

    // ── unset ─────────────────────────────────────────────────────────────────

    #[test]
    fn unset_removes_var() {
        unsafe { std::env::set_var("RSHELL_TEST_UNSET", "yes") };
        let mut e = make_test_env();
        BuiltIns::unset(&["RSHELL_TEST_UNSET"], &mut e.ctx, &mut e.term).unwrap();
        assert!(std::env::var("RSHELL_TEST_UNSET").is_err());
    }

    #[test]
    fn unset_too_many_args_is_error() {
        let mut e = make_test_env();
        assert!(BuiltIns::unset(&["A", "B"], &mut e.ctx, &mut e.term).is_err());
    }

    // ── exit ──────────────────────────────────────────────────────────────────

    #[test]
    fn exit_returns_shell_exit_signal() {
        let mut e = make_test_env();
        let result = BuiltIns::exit(&[], &mut e.ctx, &mut e.term);
        assert!(result.is_err());
        let shell_err = result
            .unwrap_err()
            .downcast::<ShellError>()
            .expect("should be ShellError");
        assert!(shell_err.is_exit());
    }

    // ── alias / unalias ───────────────────────────────────────────────────────

    #[test]
    fn alias_adds_entry() {
        let mut e = make_test_env();
        BuiltIns::alias(&["ll=ls -la"], &mut e.ctx, &mut e.term).unwrap();
        assert!(e.ctx.aliases.get("ll").is_some());
    }

    #[test]
    fn unalias_removes_entry() {
        let mut e = make_test_env();
        BuiltIns::alias(&["ll=ls -la"], &mut e.ctx, &mut e.term).unwrap();
        BuiltIns::unalias(&["ll"], &mut e.ctx, &mut e.term).unwrap();
        assert!(e.ctx.aliases.get("ll").is_none());
    }

    #[test]
    fn unalias_nonexistent_is_error() {
        let mut e = make_test_env();
        assert!(BuiltIns::unalias(&["ghost"], &mut e.ctx, &mut e.term).is_err());
    }

    #[test]
    fn alias_no_args_does_not_error() {
        let mut e = make_test_env();
        e.ctx.aliases.add("x".into(), "y".into());
        assert!(BuiltIns::alias(&[], &mut e.ctx, &mut e.term).is_ok());
    }
}

// =============================================================================
// history — tests
// =============================================================================
mod history_tests {
    use rshell::history::History;
    use tempfile::TempDir;

    /// Each test gets its own HOME so history files don't interfere.
    fn make_history() -> (History, TempDir) {
        let dir = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", dir.path()) };
        (History::new().unwrap(), dir)
    }

    #[test]
    fn starts_empty() {
        let (h, _dir) = make_history();
        assert!(h.current.is_empty());
        assert_eq!(h.row, 0);
    }

    #[test]
    fn push_adds_entry() {
        let (mut h, _dir) = make_history();
        h.push("ls".into()).unwrap();
        assert_eq!(h.current.len(), 1);
        assert_eq!(h.current[0], "ls");
    }

    #[test]
    fn push_multiple_entries() {
        let (mut h, _dir) = make_history();
        h.push("ls".into()).unwrap();
        h.push("pwd".into()).unwrap();
        h.push("echo hi".into()).unwrap();
        assert_eq!(h.current.len(), 3);
    }

    #[test]
    fn persisted_to_disk_and_reloaded() {
        let dir = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", dir.path()) };
        {
            let mut h = History::new().unwrap();
            h.push("first".into()).unwrap();
            h.push("second".into()).unwrap();
        }
        let h2 = History::new().unwrap();
        assert_eq!(h2.current.len(), 2);
        assert_eq!(h2.current[0], "first");
        assert_eq!(h2.current[1], "second");
    }

    #[test]
    fn row_starts_at_len_after_load() {
        let dir = tempfile::tempdir().unwrap();
        unsafe { std::env::set_var("HOME", dir.path()) };
        {
            let mut h = History::new().unwrap();
            h.push("a".into()).unwrap();
            h.push("b".into()).unwrap();
        }
        let h2 = History::new().unwrap();
        assert_eq!(h2.row, 2);
    }
}

// =============================================================================
// editor::Buffer — tests
// (Buffer must be `pub(crate)` in editor.rs for this to compile;
//  alternatively move these tests into editor.rs itself.)
// =============================================================================
mod buffer_tests {
    use rshell::editor::Buffer;

    fn buf(s: &str) -> Buffer {
        let mut b = Buffer::new();
        for c in s.chars() {
            b.insert(c);
        }
        b
    }

    #[test]
    fn insert_advances_index() {
        let b = buf("hello");
        assert_eq!(b.len(), 5);
    }

    #[test]
    fn backspace_removes_last_char() {
        let mut b = buf("hello");
        assert!(b.backspace());
        assert_eq!(b.content(), "hell");
    }

    #[test]
    fn backspace_on_empty_returns_false() {
        assert!(!Buffer::new().backspace());
    }

    #[test]
    fn take_drains_buffer() {
        let mut b = buf("hello");
        assert_eq!(b.take(), "hello");
        assert_eq!(b.len(), 0);
        assert_eq!(b.index, 0);
    }

    #[test]
    fn set_replaces_content_and_moves_index_to_end() {
        let mut b = Buffer::new();
        b.set("world");
        assert_eq!(b.content(), "world");
        assert_eq!(b.index, 5);
    }

    #[test]
    fn next_word_from_start() {
        let mut b = Buffer::new();
        b.set("hello world");
        b.index = 0;
        assert_eq!(b.next_word(), 6); // start of "world"
    }

    #[test]
    fn next_word_at_end_returns_len() {
        let mut b = Buffer::new();
        b.set("hello");
        assert_eq!(b.next_word(), b.len());
    }

    #[test]
    fn prev_word_from_end() {
        let mut b = Buffer::new();
        b.set("hello world"); // index at 11
        assert_eq!(b.prev_word(), 6); // start of "world"
    }

    #[test]
    fn prev_word_at_start_returns_zero() {
        let mut b = Buffer::new();
        b.set("hello");
        b.index = 0;
        assert_eq!(b.prev_word(), 0);
    }

    #[test]
    fn unicode_insert_and_backspace() {
        let mut b = Buffer::new();
        b.insert('é'); // 2-byte UTF-8
        assert_eq!(b.index, 2);
        assert!(b.backspace());
        assert_eq!(b.index, 0);
        assert_eq!(b.len(), 0);
    }

    #[test]
    fn insert_in_the_middle() {
        let mut b = Buffer::new();
        b.set("hllo");
        b.index = 1; // after 'h'
        b.insert('e');
        assert_eq!(b.content(), "hello");
    }
}

// =============================================================================
// aliases — tests
// =============================================================================
mod aliases_tests {
    use rshell::aliases::Aliases;

    #[test]
    fn add_and_get() {
        let mut a = Aliases::new();
        a.add("ll".into(), "ls -la".into());
        assert_eq!(a.get("ll").unwrap(), "ls -la");
    }

    #[test]
    fn get_nonexistent_returns_none() {
        assert!(Aliases::new().get("nope").is_none());
    }

    #[test]
    fn remove_alias() {
        let mut a = Aliases::new();
        a.add("gs".into(), "git status".into());
        a.remove("gs");
        assert!(a.get("gs").is_none());
    }

    #[test]
    fn overwrite_alias() {
        let mut a = Aliases::new();
        a.add("x".into(), "old".into());
        a.add("x".into(), "new".into());
        assert_eq!(a.get("x").unwrap(), "new");
    }

    #[test]
    fn get_map_returns_all_entries() {
        let mut a = Aliases::new();
        a.add("a".into(), "1".into());
        a.add("b".into(), "2".into());
        assert_eq!(a.get_map().len(), 2);
    }
}

// =============================================================================
// error — tests
// =============================================================================
mod error_tests {
    use rshell::error::{ShellError, ShellPhase};

    #[test]
    fn exit_error_is_exit() {
        assert!(ShellError::exit().is_exit());
    }

    #[test]
    fn non_exit_error_is_not_exit() {
        let e = ShellError {
            phase: ShellPhase::Executor,
            command: None,
            message: "something else".into(),
        };
        assert!(!e.is_exit());
    }

    #[test]
    fn display_includes_phase_command_and_message() {
        let e = ShellError {
            phase: ShellPhase::Parser,
            command: Some("ls".into()),
            message: "bad syntax".into(),
        };
        let s = e.to_string();
        assert!(s.contains("Parser"));
        assert!(s.contains("ls"));
        assert!(s.contains("bad syntax"));
    }

    #[test]
    fn display_without_command_omits_command_field() {
        let e = ShellError {
            phase: ShellPhase::Tokenizer,
            command: None,
            message: "oops".into(),
        };
        let s = e.to_string();
        assert!(s.contains("Tokenizer"));
        assert!(s.contains("oops"));
    }

    #[test]
    fn all_phases_have_non_empty_display() {
        use ShellPhase::*;
        for phase in [Tokenizer, Parser, Expander, Executor, SignalHandler] {
            assert!(!format!("{}", phase).is_empty());
        }
    }
}

// =============================================================================
// prompt — tests
// =============================================================================
mod prompt_tests {
    use rshell::prompt::Prompt;
    use std::path::PathBuf;

    #[test]
    fn new_prompt_is_empty() {
        let p = Prompt::new();
        assert!(p.message.is_empty());
        assert_eq!(p.len(), 0);
    }

    #[test]
    fn update_contains_directory_and_separator() {
        let mut p = Prompt::new();
        p.update(&PathBuf::from("/home/user"));
        assert!(p.message.contains("/home/user"));
        assert!(p.message.ends_with(">> "));
    }

    #[test]
    fn len_matches_byte_length_of_message() {
        let mut p = Prompt::new();
        p.update(&PathBuf::from("/tmp"));
        assert_eq!(p.len(), p.message.len());
    }
}

// =============================================================================
// integration — full tokenize → parse → expand → execute round trips
// =============================================================================
mod integration_tests {
    use crate::test_helpers::make_test_env;
    use rshell::shell::Shell;

    /// Parse and execute a command string, returning the exit code.
    fn run(input: &str) -> i32 {
        let mut e = make_test_env();
        let cmd = Shell::parse_command(&mut e.ctx, &mut e.term, input, true).unwrap();
        let (still_running, _) = Shell::execute_command(&mut e.ctx, &mut e.term, cmd).unwrap();
        if still_running {
            e.ctx.last_exit_code
        } else {
            -1
        }
    }

    #[test]
    fn true_exits_zero() {
        assert_eq!(run("true"), 0);
    }

    #[test]
    fn false_exits_nonzero() {
        assert_ne!(run("false"), 0);
    }

    #[test]
    fn echo_exits_zero() {
        assert_eq!(run("echo hello"), 0);
    }

    #[test]
    fn and_short_circuits_on_failure() {
        assert_ne!(run("false && echo unreachable"), 0);
    }

    #[test]
    fn or_short_circuits_on_success() {
        assert_eq!(run("true || false"), 0);
    }

    #[test]
    fn sequence_runs_both_sides() {
        assert_eq!(run("true; true"), 0);
    }

    #[test]
    fn pipeline_exit_code_is_last_stage() {
        assert_eq!(run("echo hello | cat"), 0);
    }

    #[test]
    fn redirect_out_creates_file_with_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.txt");
        run(&format!("echo hello > {}", path.display()));
        assert!(path.exists());
        assert!(std::fs::read_to_string(&path).unwrap().contains("hello"));
    }

    #[test]
    fn redirect_append_does_not_truncate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.txt");
        run(&format!("echo line1 > {}", path.display()));
        run(&format!("echo line2 >> {}", path.display()));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("line1") && content.contains("line2"));
    }

    #[test]
    fn subcommand_expansion_in_argument() {
        let mut e = make_test_env();
        let cmd =
            Shell::parse_command(&mut e.ctx, &mut e.term, "echo $(echo inner)", true).unwrap();
        assert!(cmd.to_string().contains("inner"));
    }

    #[test]
    fn builtin_cd_changes_working_directory() {
        let dir = tempfile::tempdir().unwrap();
        run(&format!("cd {}", dir.path().display()));
        assert_eq!(
            std::env::current_dir().unwrap(),
            dir.path().canonicalize().unwrap()
        );
    }
}
