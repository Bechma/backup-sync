# Project Context

## Purpose

The idea of this project is to create a synchronization mechanism for folders among different devices and operation systems.

This mechanism have a server as the centralized coordinator piece, and the agents(in the clients) that will be on charge of applying the changes received from the server or to inform the server about the changes in the local machine.

It will be possible that a bunch of agents keep in sync, thanks to the server, the same folder. And the conflict resolution will be dealt with by the server.

## Tech Stack

- Rust will be used as the main language
- Postgres will be used as the database
- Tauri will be the used for the creation of the agent as a desktop application.
- A web server will be used to interact with the server to configure the options and the agents.
- A websocket server will be used to interact with the agents to inform about file/folder changes.

## Project Conventions

### Code Style

- All the code will be written in rust
- The code will be formatted using rustfmt by using the command that will automatically lint the code `cargo fmt --all`
- The code will be linted using clippy, no errors or warnings allowed `cargo clippy`
- New code needs to be covered by tests that includes minimum the happy path, possible edge cases and error cases that are risen correctly.
- If a piece of code is repeated in more than one place, it should be extracted to a utility function or module.
- The code should be clean and simple, by the use of utility functions and/or modules to avoid code duplication.
- The code should be easy to read and understand, by the use of clear variable names and comments.

### Architecture Patterns

I want to use the KISS principle, so I don't want to overcomplicate the code.
Also, do not include features that are not strictly necessary for the project.

The organization of the project will be based on `Cargo.toml` workspace members, so it will be as follows:
- `@/agent`: The agent code that will be on charge of applying the changes received from the server or to inform the server about the changes in the local machine.
- `@/server`: The server that will be on charge of coordinating the changes between the agents. This will include the web server as well as the websocket endpoint for the agent coordination.
- `@/client/desktop`: The application that will include the implementation of the agent interaction plus a simple ui to do a basic interaction with the server.

### Testing Strategy

When creating new code, it is mandatory to cover it with tests. The tests should include at least:

- The happy path: The input and output is what we expect without any side effects.
- Possible edge cases: For an input that stress the logic, the output should remain valid or error should be risen.
- Error cases: For an invalid input, the output should be indeed an error.

Keep the test code clean and simple, by the use of utility functions to avoid code duplication.

When building functionality that can handle arbitrary inputs, consider the inclusion of proptests.

### Git Workflow

For now we won't use any specific git workflow.

## Domain Context

In order to reduce bandwidth usage, we implement a delta-based transfer for modified files.
Instead of using a custom delta library, we will use the libsync3.

## Important Constraints

For now, there are no constraints.

## External Dependencies

The list is not exhaustive:

- anyhow for the error handling
- axum for the http server
- notify to watch the file system
- postcard for the serialization and deserialization of the messages that comes and go in the websocket
- serde as the framework for the serialization and deserialization
- sqlx for the interaction with the database
- tokio for the async runtime
- tokio-tungstenite for the interaction with the websocket