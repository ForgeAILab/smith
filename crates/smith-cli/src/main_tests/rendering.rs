// rendering behavior tests.

    #[test]
    fn stdin_prompts_are_non_empty_utf8_and_bounded() {
        assert_eq!(read_prompt(&b"hello"[..]).expect("a prompt"), "hello");
        assert!(read_prompt(&b"   \n"[..]).is_err());
        assert!(read_prompt(&[0xff][..]).is_err());
        assert!(read_prompt(vec![b'x'; MAX_STDIN_PROMPT_BYTES + 1].as_slice()).is_err());
    }

    #[test]
    fn the_production_cli_manifest_uses_the_full_facade_only_as_a_dev_dependency() {
        let manifest = include_str!("../../Cargo.toml");
        let dependencies = manifest
            .split("[dev-dependencies]")
            .next()
            .expect("a dependencies section");
        assert!(
            !dependencies
                .lines()
                .any(|line| line.trim_start().starts_with("agent-runtime =")),
            "smith-cli must compose the full facade through smith-runtime"
        );
    }
