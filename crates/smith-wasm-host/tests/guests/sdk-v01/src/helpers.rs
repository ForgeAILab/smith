use crate::bindings::smith::module::types::*;

/// Builds a guest tool declaration; declarations never grant host authority.
pub struct ToolSpecBuilder(ToolSpec);
impl ToolSpecBuilder {
    /// Creates a tool with a JSON Schema parameter object.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        schema: impl Into<String>,
    ) -> Self {
        Self(ToolSpec {
            name: name.into(),
            description: description.into(),
            parameters_json: schema.into(),
            effects: vec![],
            permission_upper_bound: vec![],
        })
    }
    /// Adds a declared effect.
    pub fn effect(mut self, effect: Effect) -> Self {
        self.0.effects.push(effect);
        self
    }
    /// Adds a permission upper bound.
    pub fn permission(mut self, permission: Permission) -> Self {
        self.0.permission_upper_bound.push(permission);
        self
    }
    /// Finishes the declaration.
    pub fn build(self) -> ToolSpec {
        self.0
    }
}
/// Builds a bounded status declaration (the host independently validates it).
pub struct StatusItemBuilder(StatusItem);
impl StatusItemBuilder {
    /// Names the registered status item and its current label.
    pub fn new(name: impl Into<String>, label: impl Into<String>) -> Self {
        Self(StatusItem {
            name: name.into(),
            label: label.into(),
            severity: None,
        })
    }
    /// Sets presentation severity.
    pub fn severity(mut self, severity: StatusSeverity) -> Self {
        self.0.severity = Some(severity);
        self
    }
    /// Finishes the declaration.
    pub fn build(self) -> StatusItem {
        self.0
    }
}
/// Builds an explicit context patch with typed fragments.
#[derive(Default)]
pub struct ContextPatchBuilder(Vec<ContextFragment>);
impl ContextPatchBuilder {
    /// Appends a typed fragment.
    pub fn fragment(mut self, fragment: ContextFragment) -> Self {
        self.0.push(fragment);
        self
    }
    /// Finishes the patch.
    pub fn build(self) -> ContextPatch {
        ContextPatch { fragments: self.0 }
    }
}
