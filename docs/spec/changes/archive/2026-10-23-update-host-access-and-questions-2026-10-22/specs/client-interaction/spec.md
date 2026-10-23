## MODIFIED Requirements

### Requirement: Questionnaire has a distinct interaction surface

The interactive TUI SHALL present agent-originated questionnaires as a
temporary accessible overlay supporting bounded choices, custom free-form
answers for every agent question, explicit submit/decline, cancellation, deadline, and restored pending
state. Its responder MUST be separate from security approval. The question surface
SHALL offer an Other answer field and a separate Chat about this question action.
Discussion MUST NOT submit or fabricate a choice.

#### Scenario: User selects a design option
- **GIVEN** the active turn asks a structured design question
- **WHEN** the user selects and submits one option
- **THEN** Smith returns the typed answer to the same turn
- **AND** the answer grants no tool authority

#### Scenario: User supplies an answer outside the listed choices
- **WHEN** the model asks a question with labelled choices and omits or disables free-form answers
- **THEN** Smith still offers an Other field
- **AND** a custom answer is accepted by the broker as typed task information

#### Scenario: User discusses the question before deciding
- **WHEN** the user selects Chat about this question and types a message
- **THEN** Enter declines the unanswered questionnaire and sends that message as ordinary input to the same session
- **AND** no listed choice is selected and no answers are fabricated
- **AND** an existing composer draft is preserved
- **AND** sensitive discussion text is registered for redaction before turn continuation

#### Scenario: User returns from discussion to the question
- **WHEN** the user presses Escape while composing a discussion message
- **THEN** the original question and staged answers remain available
- **AND** nothing is sent to the model
