/*
 * Copyright (c) Facebook, Inc. and its affiliates.
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

//! An LSP-specific Compiler interface

use crate::lsp::{Connection, LSPBridgeMessage};

use relay_compiler::compiler_state::{CompilerState, ProjectName};
use relay_compiler::config::Config;
use relay_compiler::errors::Error as CompilerError;
use relay_compiler::errors::Result as CompilerResult;
use relay_compiler::FileSourceSubscription;
use relay_compiler::{build_schema, check_project, parse_sources, Programs};
use schema::Schema;

use common::{PerfLogEvent, PerfLogger};
use interner::{Intern, StringKey};

use crate::completion::{
    completion_items_for_request, get_completion_request, send_completion_response,
    GraphQLSourceCache,
};

use crate::error_reporting::{report_build_project_errors, report_syntax_errors};
use crate::state::ServerState;
use crate::text_documents::{
    on_did_change_text_document, on_did_close_text_document, on_did_open_text_document,
};

use common::ConsoleLogger;
use log::info;
use std::collections::HashMap;
use tokio::sync::mpsc::Receiver;

use tokio::select;

type SchemaMap = HashMap<ProjectName, Schema>;

pub struct LSPCompiler<'schema, 'config> {
    lsp_rx: Receiver<LSPBridgeMessage>,
    schemas: &'schema SchemaMap,
    config: &'config Config,
    subscription: FileSourceSubscription<'config>,
    compiler_state: CompilerState,
    connection: Connection,
    synced_graphql_documents: GraphQLSourceCache,
    server_state: ServerState,
    project_programs: HashMap<StringKey, Programs<'schema>>,
}

impl<'schema, 'config> LSPCompiler<'schema, 'config> {
    pub fn new(
        schemas: &'schema SchemaMap,
        config: &'config Config,
        subscription: FileSourceSubscription<'config>,
        compiler_state: CompilerState,
        lsp_rx: Receiver<LSPBridgeMessage>,
        connection: Connection,
    ) -> Self {
        let root_dir = config.root_dir.clone();
        let server_state = ServerState::new(root_dir);
        LSPCompiler {
            lsp_rx,
            config,
            schemas,
            subscription,
            compiler_state,
            connection,
            synced_graphql_documents: HashMap::new(),
            server_state,
            project_programs: HashMap::new(),
        }
    }

    async fn check_projects_and_report_errors(&mut self, event: &impl PerfLogEvent) {
        match self.check_projects(event).await {
            Ok(_) => {
                // Clear out any existing diagnostics
                self.server_state.clear_diagnostics(&self.connection);
            }
            Err(err) => {
                match err {
                    CompilerError::SyntaxErrors { errors } => {
                        report_syntax_errors(errors, &self.connection, &mut self.server_state)
                    }
                    CompilerError::BuildProjectsErrors { errors } => report_build_project_errors(
                        errors,
                        &self.connection,
                        &mut self.server_state,
                    ),
                    // Ignore the rest of these errors for now
                    CompilerError::ConfigFileRead { .. } => {}
                    CompilerError::ConfigFileParse { .. } => {}
                    CompilerError::ConfigFileValidation { .. } => {}
                    CompilerError::ReadFileError { .. } => {}
                    CompilerError::WriteFileError { .. } => {}
                    CompilerError::SerializationError { .. } => {}
                    CompilerError::DeserializationError { .. } => {}
                    CompilerError::CanonicalizeRoot { .. } => {}
                    CompilerError::Watchman { .. } => {}
                    CompilerError::EmptyQueryResult => {}
                    CompilerError::FileRead { .. } => {}
                    CompilerError::Syntax { .. } => {}
                }
            }
        }
    }

    pub async fn watch(&mut self) -> CompilerResult<()> {
        loop {
            select! {
                changes = self.subscription.next_change() => {
                    if let Ok(file_source_changes) = changes {
                        let file_source_changes = file_source_changes.unwrap();
                        let incremental_check_event =
                        ConsoleLogger.create_event("incremental_check_event");
                    let incremental_check_time =
                        incremental_check_event.start("incremental_check_time");
                    let had_new_changes = self.compiler_state.add_pending_file_source_changes(
                        &self.config,
                        &file_source_changes,
                        &incremental_check_event,
                        &ConsoleLogger,
                    )?;

                    if had_new_changes {
                        self.check_projects_and_report_errors(&incremental_check_event).await;
                    }

                    incremental_check_event.stop(incremental_check_time);
                    ConsoleLogger.complete_event(incremental_check_event);
                    // We probably don't want the messages queue to grow indefinitely
                    // and we need to flush then, as the check/build is completed
                    ConsoleLogger.flush();

                    }
                }
                message = self.lsp_rx.recv() => {
                    if let Some(message) = message {
                      self.on_lsp_bridge_message(message);
                    }
                }
            }
        }
    }

    fn on_lsp_bridge_message(&mut self, message: LSPBridgeMessage) {
        match message {
            // Completion request
            LSPBridgeMessage::CompletionRequest { params, request_id } => {
                if let Some(completion_request) =
                    get_completion_request(params, &self.synced_graphql_documents)
                {
                    info!("completion_request {:#?}", self.project_programs.keys());
                    // TODO(brandondail) don't hardcode schema here
                    let project_key = "facebook-test".intern();
                    let schema = self.schemas.get(&project_key).unwrap();

                    let programs = self.project_programs.get(&project_key);

                    info!("programs? {:?}", programs.is_some());

                    if let Some(items) =
                        completion_items_for_request(completion_request, schema, programs)
                    {
                        send_completion_response(items, request_id, &self.connection);
                    }
                }
            }
            LSPBridgeMessage::DidOpenTextDocument(params) => {
                on_did_open_text_document(params, &mut self.synced_graphql_documents);
            }
            LSPBridgeMessage::DidChangeTextDocument(params) => {
                on_did_change_text_document(params, &mut self.synced_graphql_documents);
            }
            LSPBridgeMessage::DidCloseTextDocument(params) => {
                on_did_close_text_document(params, &mut self.synced_graphql_documents);
            }
        }
    }

    pub fn build_schemas(
        config: &Config,
        compiler_state: &CompilerState,
        setup_event: &impl PerfLogEvent,
    ) -> SchemaMap {
        let timer = setup_event.start("build_schemas");
        let mut schemas = HashMap::new();
        config.for_each_project(|project_config| {
            let schema = build_schema(compiler_state, project_config);
            schemas.insert(project_config.name, schema);
        });
        setup_event.stop(timer);
        schemas
    }

    async fn check_projects(&mut self, setup_event: &impl PerfLogEvent) -> CompilerResult<()> {
        let graphql_asts =
            setup_event.time("parse_sources_time", || parse_sources(&self.compiler_state))?;
        let mut check_project_errors = vec![];
        let mut project_programs = HashMap::new();
        match self.config.only_project {
            Some(project_key) => {
                let project_config =
                    self.config.projects.get(&project_key).unwrap_or_else(|| {
                        panic!("Expected the project {} to exist", &project_key)
                    });
                let schema = self.schemas.get(&project_config.name).unwrap();
                let programs = check_project(
                    project_config,
                    &self.compiler_state,
                    &graphql_asts,
                    schema,
                    &ConsoleLogger,
                )
                .await
                .map_err(|err| {
                    check_project_errors.push(err);
                });
                if let Ok(programs) = programs {
                    project_programs.insert(project_key, programs);
                }
            }
            None => {
                for project_config in self.config.projects.values() {
                    if self
                        .compiler_state
                        .project_has_pending_changes(project_config.name)
                    {
                        let schema = self.schemas.get(&project_config.name).unwrap();
                        // TODO: consider running all projects in parallel
                        let programs = check_project(
                            project_config,
                            &self.compiler_state,
                            &graphql_asts,
                            schema,
                            &ConsoleLogger,
                        )
                        .await
                        .map_err(|err| {
                            check_project_errors.push(err);
                        })
                        .ok();
                        if let Some(programs) = programs {
                            project_programs.insert(project_config.name, programs);
                        }
                    }
                }
            }
        }

        if check_project_errors.is_empty() {
            self.project_programs = project_programs;
            Ok(())
        } else {
            Err(CompilerError::BuildProjectsErrors {
                errors: check_project_errors,
            })
        }
    }
}
