# Scenario traceability

One row per scenario in this change's deltas. `Layer` is the test layer
from `design.md` (1 runtime conformance, 2 bridge units, 3 endpoint,
4 stand-in end to end, 5 surfaces, 6 live). `Test` names each test as
`package / test target::path`; a test is listed only where it asserts the
scenario's THEN and AND lines and passes. Task 8.3 closes when no `Test`
cell is empty. Scenarios kept unchanged inside a MODIFIED requirement are
listed too, against the tests that already cover them.

## advisor

| Requirement | Scenario | Layer | Test |
| --- | --- | --- | --- |
| Advisor on a harness session | Mid-turn call from a harness | 2, 4 | `smith-runtime / lib::advisor::tests::harness_advisor_request_orders_cli_tools_and_canonical_bridge_rounds`<br>`smith-cli / cli_contract::bridge_advisor::recording_advisor_contains_mid_turn_transcript_and_attributed_usage` |
| Advisor on a harness session | The CLI's own tools are off | 2, 4 | `smith-runtime / lib::advisor::tests::harness_advisor_without_own_tools_is_byte_identical_to_native_history`<br>`smith-cli / cli_contract::bridge_advisor::recording_advisor_contains_mid_turn_transcript_and_attributed_usage` |
| Advisor on a harness session | No advisor configured | 2, 4 | `smith-cli / cli_contract::bridge_advisor::no_advisor_omits_tool_and_guidance` |

## child-agents

| Requirement | Scenario | Layer | Test |
| --- | --- | --- | --- |
| Delegation from a harness session | A harness parent spawns a provider child | 4 | `smith-cli / cli_contract::bridge_agents::spawn_obeys_native_delegation_approval_and_refusal_creates_no_child` |
| Delegation from a harness session | The CLI waits for its child | 4 | `smith-cli / cli_contract::bridge_agents::provider_child_wait_is_acknowledged_and_all_actions_round_trip` |
| Delegation from a harness session | The CLI does not wait | 4 | `smith-cli / cli_contract::bridge_agents::unread_provider_outcome_gets_one_internal_resumed_harness_turn` |
| Delegation from a harness session | Delegation disabled on the profile | 4 | `smith-cli / cli_contract::bridge_agents::delegation_disabled_removes_tool_and_guidance` |
| Delegation from a harness session | A harness child cannot delegate | 4 | `smith-cli / cli_contract::bridge_agents::harness_child_receives_neither_agent_nor_advisor` |

## client-surfaces

| Requirement | Scenario | Layer | Test |
| --- | --- | --- | --- |
| Bridged calls render as Smith tool calls | A bridged call in the TUI | 4, 5 | `smith-tui / lib::app::state::tests::bridge::mixed_harness_turn_preserves_text_order_and_one_row_per_owner`<br>`smith-tui / lib::render::layout::tests::bridge::mixed_harness_rows_render_once_with_native_smith_detail_and_cli_origin`<br>`smith-tui / bridge_surfaces::live_headless_harness_stream_and_tui_have_the_same_tools_text_and_outcome` |
| Bridged calls render as Smith tool calls | A bridged call in headless machine output | 4, 5 | `smith-cli / cli_contract::bridge_surfaces::stream_json_bridged_agent_is_an_ordinary_pair_beside_one_cli_pair`<br>`smith-cli / cli_contract::bridge_surfaces::json_bridged_agent_has_one_successful_result_and_final_text` |
| Bridged calls render as Smith tool calls | An approval for a bridged call | 4, 5 | `smith-tui / lib::render::layout::tests::bridge::bridged_spawn_approval_matches_native_action_prompt`<br>`smith-cli / cli_contract::bridge_surfaces::bridged_spawn_fails_closed_with_the_native_headless_approval_result` |
| Bridged calls render as Smith tool calls | The CLI never connected | 4, 5 | `smith-tui / lib::app::state::tests::bridge::bridge_diagnostic_is_visible_and_successful_terminal_stays_completed`<br>`smith-tui / lib::render::layout::tests::bridge::bridge_diagnostic_renders_without_a_failed_turn`<br>`smith-cli / cli_contract::bridge_surfaces::claude_never_connected_diagnostic_survives_all_formats_without_failure` |

## configuration

| Requirement | Scenario | Layer | Test |
| --- | --- | --- | --- |
| CLI-owned tools are off by default | Default harness turn runs without CLI tools | 2, 4 | `smith-runtime / lib::cli_agent::invocation::tests::claude_runs_without_its_own_tools_by_default`<br>`smith-runtime / lib::cli_agent::invocation::tests::codex_runs_read_only_by_default_and_writes_only_when_permitted` |
| CLI-owned tools are off by default | Project cannot enable CLI tools | 2, 4 | `smith-cli / cli_contract::bridge_persistence::project_cannot_enable_owner_controlled_cli_tools` |
| CLI-owned tools are off by default | Bridge with CLI tools off | 2, 4 | `smith-runtime / lib::cli_agent::invocation::tests::claude_no_own_tools_first_bridged`<br>`smith-runtime / lib::cli_agent::invocation::tests::codex_no_own_tools_first_bridged`<br>`smith-cli / cli_contract::bridge::claude_bridged_agent_call_and_resume_commit_events_and_history`<br>`smith-cli / cli_contract::bridge::codex_bridged_agent_call_and_resume_commit_events_and_history` |
| CLI-owned tools are off by default | Denying every capability turns the bridge off | 2, 4 | `smith-cli / cli_contract::bridge::claude_deny_tools_and_skills_omits_the_bridge`<br>`smith-cli / cli_contract::bridge::codex_deny_tools_and_skills_omits_the_bridge` |

## extension-system

| Requirement | Scenario | Layer | Test |
| --- | --- | --- | --- |
| Smith-declared MCP servers reach a harness only through Smith | A remote tool called from a harness | 4 | `smith-cli / cli_contract::bridge_extensions::trusted_remote_call_executes_once_with_credential_kept_out_of_cli` |
| Smith-declared MCP servers reach a harness only through Smith | The CLI's invocation is inspected | 4 | `smith-cli / cli_contract::bridge_extensions::trusted_remote_call_executes_once_with_credential_kept_out_of_cli` |
| Smith-declared MCP servers reach a harness only through Smith | An untrusted server | 4 | `smith-cli / cli_contract::bridge_extensions::untrusted_server_is_neither_listed_nor_callable` |
| Smith-declared MCP servers reach a harness only through Smith | A module tool | 4 | `smith-cli / cli_contract::bridge_extensions::installed_module_tool_runs_under_confirmed_workspace_grants` |

## provider-runtime

| Requirement | Scenario | Layer | Test |
| --- | --- | --- | --- |
| Harness turns are offered Smith capabilities through a turn-scoped bridge | A harness session with delegation enabled | 2, 4 | `smith-cli / cli_contract::bridge::claude_bridged_agent_call_and_resume_commit_events_and_history` |
| Harness turns are offered Smith capabilities through a turn-scoped bridge | Nothing to bridge | 2, 4 | `smith-cli / cli_contract::bridge::claude_deny_tools_and_skills_omits_the_bridge`<br>`smith-cli / cli_contract::bridge::codex_deny_tools_and_skills_omits_the_bridge`<br>`smith-runtime / lib::cli_agent::process_tests::claude_disabled_bridge_preserves_the_launchers_exact_argv`<br>`smith-runtime / lib::cli_agent::process_tests::codex_disabled_bridge_preserves_the_launchers_exact_argv` |
| Harness turns are offered Smith capabilities through a turn-scoped bridge | A resumed CLI session | 2, 4 | `smith-cli / cli_contract::bridge::claude_bridged_agent_call_and_resume_commit_events_and_history`<br>`smith-cli / cli_contract::bridge::codex_bridged_agent_call_and_resume_commit_events_and_history`<br>`smith-cli / cli_contract::bridge_persistence::saved_rounds_journal_listing_and_resume_use_a_fresh_working_endpoint` |
| Harness turns are offered Smith capabilities through a turn-scoped bridge | The endpoint cannot start | 2, 4 | `smith-runtime / lib::cli_agent::process_tests::endpoint_bind_failure_fails_turn_before_cli_start` |
| The bridged set excludes what the CLI replaces | Only support tools remain | 2, 4 | `smith-runtime / lib::cli_bridge::tests::authorized_session_selection_matrix`<br>`smith-runtime / lib::cli_bridge::tests::artifact_reader_cannot_survive_the_last_valid_capability` |
| The bridged set excludes what the CLI replaces | Core tools stay with the CLI | 2, 4 | `smith-runtime / lib::cli_bridge::tests::authorized_session_selection_matrix`<br>`smith-cli / cli_contract::bridge::claude_bridged_agent_call_and_resume_commit_events_and_history` |
| The bridged set excludes what the CLI replaces | A denied capability | 2, 4 | `smith-cli / cli_contract::bridge_agents::denied_agent_is_unlisted_and_refused_without_dispatch_or_child`<br>`smith-runtime / lib::cli_bridge::failure_tests::unknown_tool_is_refused_without_dispatch` |
| The bridged set excludes what the CLI replaces | A child on a harness | 2, 4 | `smith-runtime / lib::cli_bridge::tests::authorized_session_selection_matrix`<br>`smith-cli / cli_contract::bridge_agents::harness_child_receives_neither_agent_nor_advisor` |
| Bridged calls are dispatched as Smith tool calls | An approval outlasts the CLI's default timeout | 1, 2, 4 | `smith-cli / cli_contract::bridge_approvals::approval_outlasts_default_timeout_in_both_dialects`<br>`smith-cli / cli_contract::stand_in::default_tool_timeout_fires_when_configured_timeout_is_ignored` |
| Bridged calls are dispatched as Smith tool calls | A bridged call needs approval | 1, 2, 4 | `agent-runtime / external_agent::approval_is_requested_before_execution_and_denial_does_not_fail_the_turn`<br>`smith-cli / cli_contract::bridge_approvals::bridged_remote_call_waits_for_interactive_server_attributed_approval` |
| Bridged calls are dispatched as Smith tool calls | Approval is denied | 1, 2, 4 | `agent-runtime / external_agent::approval_is_requested_before_execution_and_denial_does_not_fail_the_turn`<br>`smith-runtime / lib::cli_bridge::failure_tests::dispatcher_denial_and_schema_failure_are_tool_errors`<br>`smith-cli / cli_contract::bridge_extensions::remote_approval_asks_with_server_attribution_and_denial_has_no_effect` |
| Bridged calls are dispatched as Smith tool calls | Headless run without an approver | 1, 2, 4 | `smith-cli / cli_contract::bridge_extensions::remote_approval_asks_with_server_attribution_and_denial_has_no_effect`<br>`smith-cli / cli_contract::bridge_agents::spawn_obeys_native_delegation_approval_and_refusal_creates_no_child` |
| Bridged calls are dispatched as Smith tool calls | A question with nobody to answer it | 1, 2, 4 | `smith-cli / cli_contract::bridge_outputs::headless_run_offers_no_questionnaire_and_refuses_a_call_to_it` |
| Bridged calls are dispatched as Smith tool calls | The CLI gives up on a call | 1, 2, 4 | `agent-runtime / external_agent::dropping_an_approval_wait_withdraws_it_and_a_late_grant_cannot_run`<br>`agent-runtime / external_agent::dropping_a_broker_wait_closes_the_real_interaction_without_a_tool_effect`<br>`smith-cli / cli_contract::bridge_approvals::cli_cancellation_withdraws_approval_and_late_allow_runs_nothing` |
| Bridged calls are dispatched as Smith tool calls | The turn is interrupted | 1, 2, 4 | `agent-runtime / external_agent::execution::cancellation_during_execution_observes_the_turn_and_matches_provider_history`<br>`agent-runtime / external_agent::execution::output_processing::cancelled_execution_bypasses_blocking_processor_in_provider_and_bridge`<br>`smith-cli / cli_contract::bridge_failures::headless_interrupt_cancels_call_and_stops_harness_process_group` |
| The bridge endpoint is private to one turn | Another local user | 2, 3 | `smith-runtime / lib::cli_bridge::failure_tests::peer_uid_mismatch_is_dropped_before_handshake` |
| The bridge endpoint is private to one turn | The turn ends | 2, 3 | `smith-runtime / lib::cli_bridge::failure_tests::private_modes_and_close_or_drop_remove_the_endpoint` |
| The bridge endpoint is private to one turn | Argument list is inspected | 2, 3 | `smith-runtime / lib::cli_agent::invocation::tests::claude_no_own_tools_first_bridged`<br>`smith-runtime / lib::cli_agent::invocation::tests::codex_no_own_tools_first_bridged`<br>`smith-cli / cli_contract::bridge_extensions::trusted_remote_call_executes_once_with_credential_kept_out_of_cli` |
| Bridged tool names are stable and unambiguous | A dotted name | 2, 4 | `smith-runtime / lib::cli_bridge::tests::registered_names_and_full_specs_pass_through_unchanged`<br>`smith-cli / cli_contract::bridge::claude_bridged_agent_call_and_resume_commit_events_and_history`<br>`smith-cli / cli_contract::bridge::codex_bridged_agent_call_and_resume_commit_events_and_history` |
| Bridged tool names are stable and unambiguous | Two names collide for the CLI | 2, 4 | `smith-runtime / lib::cli_bridge::tests::collisions_omit_every_member_and_report_each_group_once` |
| Bridged capability guidance reaches the CLI | Delegation guidance follows the tool | 2, 4 | `smith-runtime / lib::cli_bridge::tests::guidance_contains_exactly_the_bridged_fragments_in_authored_order`<br>`smith-runtime / lib::cli_bridge::tests::guidance_names_only_capabilities_that_are_bridged`<br>`smith-cli / cli_contract::bridge_agents::delegation_disabled_removes_tool_and_guidance` |
| Bridged capability guidance reaches the CLI | The largest bridged set | 2, 4 | `smith-runtime / lib::cli_bridge::tests::guidance_drops_whole_trailing_fragments_and_counts_characters` |
| Bridged capability guidance reaches the CLI | A capability is removed from a running conversation | 2, 4 | `smith-cli / cli_contract::bridge_advisor::removing_advisor_on_resume_preserves_recorded_cli_instructions` |
| Skills reach a harness as activation results | A skill is activated from a harness | 1, 4 | `agent-runtime / external_agent::extensions::external_activation_returns_verified_instructions_without_staging_or_spending_budget`<br>`smith-cli / cli_contract::bridge_skills::trusted_skill_is_discovered_and_its_instructions_return_over_mcp` |
| Skills reach a harness as activation results | A skill's content changed | 1, 4 | `smith-cli / cli_contract::bridge_skills::edited_skill_fails_pinned_activation_and_returns_no_instruction_body` |
| Interrupting a harness turn stops the CLI | The CLI is a launcher around the agent | 2, 4 | `smith-runtime / lib::cli_agent::process_tests::cancelling_a_turn_stops_the_launcher_and_its_child`<br>`smith-runtime / lib::cli_agent::process_tests::dropping_a_turn_stream_stops_the_launcher_and_its_child`<br>`smith-cli / cli_contract::bridge_failures::headless_interrupt_cancels_call_and_stops_harness_process_group` |
| Interrupting a harness turn stops the CLI | Smith's process is killed | 2, 4 | `smith-cli / cli_contract::bridge_failures::killing_smith_mid_turn_stops_harness_and_child_without_graceful_shutdown`<br>`smith-cli / cli_contract::harness_supervise::sigkill_of_the_parent_holding_stdin_stops_the_supervisor_group`<br>`smith-cli / cli_contract::harness_supervise::stdin_eof_kills_supervisor_program_and_grandchild` |
| Interrupting a harness turn stops the CLI | A headless run is interrupted from the terminal | 2, 4 | `smith-cli / cli_contract::harness_supervise::each_headless_signal_reports_cancelled_and_saves_the_interrupted_call`<br>`smith-cli / cli_contract::bridge_failures::headless_interrupt_cancels_call_and_stops_harness_process_group` |
| Interrupting a harness turn stops the CLI | An interrupted turn stops calling Smith | 2, 4 | `smith-cli / cli_contract::bridge_failures::headless_interrupt_cancels_call_and_stops_harness_process_group` |
| Capability bridge tests | Bridge matrix runs without a CLI | 4, 6 | `smith-cli / cli_contract::bridge::bridge_turns_run_with_neither_cli_on_path` |
| Capability bridge tests | A wrong server declaration | 4, 6 | `smith-cli / cli_contract::stand_in::malformed_declarations_fail_before_stdout_or_wire_activity` |
| Capability bridge tests | An abandoned call is observed, not assumed | 4, 6 | `smith-cli / cli_contract::bridge_approvals::connection_abandonment_is_observed_before_late_approval_runs_nothing`<br>`agent-runtime / external_agent::dropping_an_approval_wait_withdraws_it_and_a_late_grant_cannot_run` |
| Capability bridge tests | A CLI release changes its output | 4, 6 | `smith-cli / live_cli_bridge::claude_drift_canary (opt-in)`<br>`smith-cli / live_cli_bridge::codex_drift_canary (opt-in)`<br>`smith-cli / live_cli_bridge::recorded_streams_retain_the_fields_compared_by_the_canary` |
| Bridged calls are not reported twice | The CLI echoes a bridged call | 2 | `smith-runtime / lib::cli_agent::fixture_tests::claude_bridge_echo_is_suppressed_beside_its_own_tools_and_outcomes`<br>`smith-runtime / lib::cli_agent::fixture_tests::codex_suppresses_the_bridge_echo_and_keeps_its_own_command` |
| Bridged calls are not reported twice | The CLI runs its own tool in the same turn | 2 | `smith-runtime / lib::cli_agent::fixture_tests::claude_bridge_echo_is_suppressed_beside_its_own_tools_and_outcomes`<br>`smith-runtime / lib::cli_agent::fixture_tests::codex_suppresses_the_bridge_echo_and_keeps_its_own_command` |

## runtime-integration

| Requirement | Scenario | Layer | Test |
| --- | --- | --- | --- |
| Child completion uses conditional internal turns | Idle parked session accepts child completion | 1, 4 | `agent-runtime / external_agent::internal_turns::child_completion_without_backend_keeps_the_provider_loop`<br>`smith-runtime / delegation::completion::a_child_spawned_inside_a_parent_turn_is_delivered_without_another_user_turn` |
| Child completion uses conditional internal turns | Idle parked harness session accepts child completion | 1, 4 | `agent-runtime / external_agent::internal_turns::child_completion_uses_backend_bridge_and_replays_without_redelivery`<br>`smith-cli / cli_contract::bridge_agents::unread_provider_outcome_gets_one_internal_resumed_harness_turn` |
| Child completion uses conditional internal turns | Parent became busy before admission | 1, 4 | `agent-runtime / external_agent::internal_turns::busy_and_stale_child_admission_match_provider_sessions` |
| Child completion uses conditional internal turns | Continuation history is inspected | 1, 4 | `agent-runtime / external_agent::internal_turns::child_completion_uses_backend_bridge_and_replays_without_redelivery` |
| Child completion uses conditional internal turns | Replay sees a committed child continuation | 1, 4 | `agent-runtime / external_agent::extensions::interrupted_child_completion_keeps_the_provider_contract_of_durable_consumption_once`<br>`agent-runtime / external_agent::internal_turns::interrupted_internal_external_checkpoints_never_resume_provider_or_bridge` |
| External turns dispatch host tools only through Agent Runtime | A bridged call during an external turn | 1, 4 | `agent-runtime / external_agent::bridge_calls_commit_text_and_two_rounds_without_provider_attempts` |
| External turns dispatch host tools only through Agent Runtime | A tool that was never activated | 1, 4 | `agent-runtime / external_agent::bridge_lists_authorized_tools_in_registration_order_without_activation` |
| External turns dispatch host tools only through Agent Runtime | History after a bridged call | 1, 4 | `agent-runtime / external_agent::bridge_calls_commit_text_and_two_rounds_without_provider_attempts`<br>`smith-cli / cli_contract::bridge_persistence::saved_rounds_journal_listing_and_resume_use_a_fresh_working_endpoint` |
| External turns dispatch host tools only through Agent Runtime | A call after the turn ended | 1, 4 | `agent-runtime / external_agent::bridge_calls_commit_text_and_two_rounds_without_provider_attempts`<br>`agent-runtime / external_agent::disabled_bridge_is_send_sync_and_refuses_without_a_runtime` |
| External turns dispatch host tools only through Agent Runtime | Process ends during a bridged call | 1, 4 | `agent-runtime / external_agent::extensions::process_exit_in_each_external_phase_interrupts_without_replay_and_accepts_new_work` |
| Internal turns on a harness session run on the harness | A child completes while a harness parent is idle | 1, 4 | `agent-runtime / external_agent::internal_turns::child_completion_uses_backend_bridge_and_replays_without_redelivery`<br>`smith-cli / cli_contract::bridge_agents::unread_provider_outcome_gets_one_internal_resumed_harness_turn` |
| Internal turns on a harness session run on the harness | Continuation history on a harness session | 1, 4 | `agent-runtime / external_agent::internal_turns::child_completion_uses_backend_bridge_and_replays_without_redelivery`<br>`smith-cli / cli_contract::bridge_agents::unread_provider_outcome_gets_one_internal_resumed_harness_turn` |
| Internal turns on a harness session run on the harness | Replay after a committed harness continuation | 1, 4 | `agent-runtime / external_agent::internal_turns::child_completion_uses_backend_bridge_and_replays_without_redelivery`<br>`agent-runtime / external_agent::internal_turns::interrupted_internal_external_checkpoints_never_resume_provider_or_bridge` |
