use alicent_domain::CreateAgentCommand;
use alicent_project_repository::Repository;
use std::{path::Path, sync::atomic::AtomicBool};
use tempfile::TempDir;
use uuid::Uuid;

#[test]
fn initialized_agent_queue_survives_backup_reopen_and_restore() {
    let source = TempDir::new().unwrap();
    let target = TempDir::new().unwrap();
    let mut repository = Repository::create(source.path(), "Queue recovery").unwrap();
    let root = repository.root().to_owned();
    let profile_id = repository.agent_execution_profiles(1).unwrap()[0]
        .id
        .clone();
    repository
        .agent_create(
            CreateAgentCommand {
                command_id: Uuid::new_v4(),
                agent_id: Uuid::new_v4(),
                name: "Recovery agent".into(),
                instructions: "Validate queue backups".into(),
                profile_id,
            },
            2,
        )
        .unwrap();

    let backup = repository
        .backup_to(target.path(), &AtomicBool::new(false))
        .unwrap();
    drop(repository);

    let mut reopened = Repository::open(&root).unwrap();
    assert_eq!(reopened.agent_execution_profiles(3).unwrap().len(), 3);
    assert_eq!(reopened.agent_custom_agents(3).unwrap().len(), 1);
    drop(reopened);

    let mut restored = Repository::restore_backup(
        Path::new(&backup.path),
        target.path(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(restored.agent_execution_profiles(4).unwrap().len(), 3);
    assert_eq!(restored.agent_custom_agents(4).unwrap().len(), 1);
}
