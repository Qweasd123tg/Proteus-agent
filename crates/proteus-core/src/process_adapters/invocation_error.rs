use std::{error::Error, fmt};

use proteus_module_protocol::{ProcessModuleRpcError, v3::ComponentFailure};

/// Machine-readable terminal failure preserved at the process adapter boundary.
#[derive(Clone, Debug, PartialEq)]
pub enum ProcessInvocationFailure {
    Module(ProcessModuleRpcError),
    Canceled,
    TimedOut,
    ComponentLost(ComponentFailure),
}

/// Typed failure of a terminal process invocation.
///
/// Slot traits still return `anyhow::Result`, but callers can downcast the
/// error to this type and make a decision without parsing display text.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessInvocationError {
    module_id: String,
    method: String,
    failure: ProcessInvocationFailure,
}

impl ProcessInvocationError {
    pub fn new(
        module_id: impl Into<String>,
        method: impl Into<String>,
        failure: ProcessInvocationFailure,
    ) -> Self {
        Self {
            module_id: module_id.into(),
            method: method.into(),
            failure,
        }
    }

    pub fn module_id(&self) -> &str {
        &self.module_id
    }

    pub fn method(&self) -> &str {
        &self.method
    }

    pub fn failure(&self) -> &ProcessInvocationFailure {
        &self.failure
    }
}

impl fmt::Display for ProcessInvocationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prefix = format_args!("process module {:?}: {}", self.module_id, self.method);
        match &self.failure {
            ProcessInvocationFailure::Module(error) => {
                write!(formatter, "{prefix} returned an error: {error}")
            }
            ProcessInvocationFailure::Canceled => write!(formatter, "{prefix} was canceled"),
            ProcessInvocationFailure::TimedOut => write!(formatter, "{prefix} timed out"),
            ProcessInvocationFailure::ComponentLost(failure) => {
                write!(formatter, "{prefix} lost its component: {failure:?}")
            }
        }
    }
}

impl Error for ProcessInvocationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.failure {
            ProcessInvocationFailure::Module(error) => Some(error),
            ProcessInvocationFailure::Canceled
            | ProcessInvocationFailure::TimedOut
            | ProcessInvocationFailure::ComponentLost(_) => None,
        }
    }
}
