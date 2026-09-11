use a2a::{
    Message, Part, Role, SendMessageRequest, SendMessageResponse, Task, TaskState, error_code,
};
use a2a_server::{RequestHandler, ServiceParams};

use super::{A2aServerConfig, state::Service};
use crate::core::AppConfig;

fn request(context: Option<String>) -> SendMessageRequest {
    let mut message = Message::new(Role::User, vec![Part::text("test")]);
    message.context_id = context;
    SendMessageRequest {
        message,
        configuration: None,
        metadata: None,
        tenant: None,
    }
}

async fn send(service: &Service, context: Option<String>) -> Task {
    let response = service
        .send_message(&ServiceParams::default(), request(context))
        .await
        .unwrap();
    let SendMessageResponse::Task(task) = response else {
        panic!("expected task")
    };
    task
}

#[tokio::test]
async fn failed_assembly_settles_and_capacity_never_evicts_task_or_context() {
    let root = tempfile::tempdir().unwrap();
    // Deliberately lacks configured process exports. A failed assembly must
    // settle the accepted task and release context admission without inference.
    let service = Service::new(
        AppConfig::default(),
        root.path().into(),
        Some(root.path().join("config.json")),
        A2aServerConfig {
            max_contexts: 1,
            max_tasks: 2,
            ..Default::default()
        },
    );
    let mut invalid = request(None);
    invalid.message.parts = vec![Part::raw(vec![0, 1])];
    let error = service
        .send_message(&ServiceParams::default(), invalid)
        .await
        .unwrap_err();
    assert_eq!(error.code, error_code::CONTENT_TYPE_NOT_SUPPORTED);
    assert!(service.registry.lock().await.contexts.is_empty());

    let first = send(&service, None).await;
    assert_eq!(first.status.state, TaskState::Failed);
    let error = service
        .send_message(&ServiceParams::default(), request(None))
        .await
        .unwrap_err();
    assert_eq!(error.code, error_code::UNSUPPORTED_OPERATION);
    assert!(error.message.contains("context capacity"));
    let second = send(&service, Some(first.context_id.clone())).await;
    assert_eq!(second.status.state, TaskState::Failed);
    assert_ne!(first.id, second.id);
    let error = service
        .send_message(
            &ServiceParams::default(),
            request(Some(first.context_id.clone())),
        )
        .await
        .unwrap_err();
    assert!(error.message.contains("task capacity"));
    assert_eq!(
        *service.lookup(&first.id).await.unwrap().updates.borrow(),
        first
    );
    assert_eq!(service.registry.lock().await.contexts.len(), 1);
    assert_eq!(service.registry.lock().await.tasks.len(), 2);
    service.shutdown().await;
    let error = service
        .send_message(&ServiceParams::default(), request(None))
        .await
        .unwrap_err();
    assert!(error.message.contains("shutting down"));
}
