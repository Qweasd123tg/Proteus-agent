use std::{path::Path, sync::Arc};

use crate::{
    contracts::{
        ApprovalPolicy, PROCESS_POLICY_CONTRACT_VERSION, PROCESS_POLICY_EVALUATE_METHOD,
        PROCESS_POLICY_VISIBILITY_METHOD, PolicyContext, PolicyVisibilityContext,
        ProcessPolicyEvaluateInput, ProcessPolicyResponse, ProcessPolicyVisibilityInput,
    },
    domain::{PolicyDecision, ToolCall},
};
use anyhow::Result;

use super::{ProcessExportClient, ProcessExportConfig};

const DEFAULT_TIMEOUT_MS: u64 = 30_000;

pub struct ProcessApprovalPolicy {
    client: Arc<ProcessExportClient>,
}

impl ProcessApprovalPolicy {
    pub fn new(config: ProcessExportConfig, workspace: &Path) -> Result<Self> {
        Ok(Self {
            client: Arc::new(ProcessExportClient::connect(
                "policy",
                PROCESS_POLICY_CONTRACT_VERSION,
                config,
                workspace,
                DEFAULT_TIMEOUT_MS,
            )?),
        })
    }
}

impl ApprovalPolicy for ProcessApprovalPolicy {
    fn evaluate(&self, call: &ToolCall, ctx: &PolicyContext) -> PolicyDecision {
        let input = ProcessPolicyEvaluateInput {
            call: call.clone(),
            cwd: ctx.cwd.clone(),
            tool_spec: ctx.tool_spec.clone(),
            granted_permissions: ctx.granted_permissions.clone(),
        };
        match self
            .client
            .invoke::<_, ProcessPolicyResponse>(PROCESS_POLICY_EVALUATE_METHOD, &input)
        {
            Ok(response) => response.result,
            Err(error) => PolicyDecision::Deny {
                reason: format!("process policy failed: {error:#}"),
            },
        }
    }

    fn evaluate_visibility(&self, ctx: &PolicyVisibilityContext) -> PolicyDecision {
        let input = ProcessPolicyVisibilityInput {
            cwd: ctx.cwd.clone(),
            tool_spec: ctx.tool_spec.clone(),
        };
        match self
            .client
            .invoke::<_, ProcessPolicyResponse>(PROCESS_POLICY_VISIBILITY_METHOD, &input)
        {
            Ok(response) => response.result,
            Err(error) => PolicyDecision::Deny {
                reason: format!("process policy failed: {error:#}"),
            },
        }
    }
}
