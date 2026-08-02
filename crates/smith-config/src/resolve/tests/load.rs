// load behavior tests.

    #[test]
    fn environment_names_follow_the_setting_key() {
        assert_eq!(env_name("model"), "SMITH_MODEL");
        assert_eq!(
            env_name("context.output_reserve"),
            "SMITH_CONTEXT_OUTPUT_RESERVE"
        );
        for (key, _) in SETTINGS {
            assert_eq!(setting_for_env(&env_name(key)), Some(*key), "{key}");
        }
    }

    #[test]
    fn keys_with_punctuation_are_quoted_so_they_round_trip() {
        assert_eq!(
            join_key(&["models", "acme/example-model", "context_tokens"]),
            "models.\"acme/example-model\".context_tokens"
        );
        assert_eq!(
            unquote_segment("\"acme/example-model\""),
            "acme/example-model"
        );
        assert_eq!(unquote_segment("plain"), "plain");
    }

    #[test]
    fn a_near_miss_is_suggested_and_a_distant_one_is_not() {
        let known = ["model", "provider", "profile"];
        assert_eq!(
            nearest("modle", known.into_iter()),
            vec!["model".to_owned()]
        );
        assert!(nearest("wildly-different", known.into_iter()).is_empty());
    }

    #[test]
    fn serde_unknown_field_messages_yield_the_key_and_its_alternatives() {
        let (key, candidates) = unknown_field(
            "unknown field `output_reserv`, expected one of `output_reserve`, `reasoning_reserve`",
        )
        .expect("an unknown-field message");
        assert_eq!(key, "output_reserv");
        assert_eq!(candidates, vec!["output_reserve", "reasoning_reserve"]);
        assert!(unknown_field("invalid type: string, expected u32").is_none());
    }

    #[test]
    fn positions_count_lines_and_columns_from_one() {
        let text = "a = 1\nbb = 2\n";
        assert_eq!(position(text, 0).to_string(), "line 1, column 1");
        assert_eq!(position(text, 6).to_string(), "line 2, column 1");
        assert_eq!(position(text, 8), Position { line: 2, column: 3 });
    }
