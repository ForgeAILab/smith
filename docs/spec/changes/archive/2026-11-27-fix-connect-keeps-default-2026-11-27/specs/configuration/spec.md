## MODIFIED Requirements

### Requirement: Additive provider and model setup

Explicit setup SHALL support adding a provider with its first model and adding
another model to an existing named provider. It MUST preserve existing
provider/model records, validate each model against that provider, and change
the default profile or model only when the user explicitly confirms that
selection. This applies equally to built-in catalog key connections. Making a
new default MUST NOT remove existing profiles from `profile_order`.

#### Scenario: Add a second model to an existing provider

- **GIVEN** user configuration declares provider `acme`
- **WHEN** the user runs `smith setup add-model --provider acme` and confirms a
  valid model with complete trusted or explicit limits
- **THEN** Smith adds the `models."acme/<model>"` record
- **AND** preserves the provider credential, endpoint, existing models, and
  unrelated configuration

#### Scenario: Add a model for another provider

- **GIVEN** multiple providers are configured
- **WHEN** the user chooses Add model without a provider argument
- **THEN** setup requires an explicit provider selection before collecting the
  model
- **AND** writes the model only under the selected provider/model identity

#### Scenario: Keep the current default

- **GIVEN** setup successfully adds a provider or model
- **WHEN** the user declines to make it the default
- **THEN** the existing default profile and active model remain unchanged
- **AND** the new configuration is available for later `/provider`, `/model`,
  or `/profile` selection

#### Scenario: Keep the current default for a catalog key connection

- **GIVEN** an install whose default profile is `code`
- **WHEN** the user connects Anthropic, Google, or xAI with an API key and
  chooses Keep current default
- **THEN** Smith writes only that provider's section and credential
- **AND** `default_profile`, `profile_order`, and every profile are unchanged
- **AND** the provider's catalog models are selectable through `/model`

#### Scenario: A new default joins the existing profile order

- **GIVEN** user configuration sets `profile_order = ["code", "sol"]`
- **WHEN** setup makes a new `claude` profile the default
- **THEN** `profile_order` becomes `["code", "sol", "claude", "claude-plan",
  "claude-review"]`
- **AND** an absent `profile_order` stays absent

#### Scenario: First run always sets the default

- **GIVEN** an empty install
- **WHEN** first-run setup connects a provider
- **THEN** setup does not offer Keep current default
- **AND** the connection's profile becomes the default
