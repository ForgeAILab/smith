from pathlib import Path
p = Path('crates/smith-cli/src/local_command.rs')
s = p.read_text()
a = '    use smith_config::model::CacheMaintenanceMode;'
assert s.count(a) == 1
p.write_text(s.replace(a, '    use smith_runtime::cache_lifecycle::CacheMaintenanceMode;'))

p = Path('crates/smith-runtime/src/host.rs')
s = p.read_text()
a = s.index('    if session.interrupted_on_resume().is_some() {', s.index('    if session.interrupted_on_resume().is_some() {') + 1)
b = s.index('\n    let goal_admission_gate', a)
s = s[:a] + '''    if session.interrupted_on_resume().is_some()
        && let Some(component) = runtime.goal_component()
        && let Some(goal) = session.goal(component)?
        && goal.status == agent_runtime_core::goal::GoalStatus::Active
    {
        session
            .control_goal(component, GoalCommand::Pause { id: goal.id, generation: goal.generation })
            .await?;
    }
''' + s[b:]
p.write_text(s)
