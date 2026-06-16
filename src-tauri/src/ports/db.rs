use crate::domain::models::{
    AgentConfig, AgentProfile, ChatMessage, ChatSession, Machine, Message, SessionHistory,
    ThreadSession, WorkingMemoryEntry, ProviderInstance, Project, Repository, Feature,
    ProjectSettings,
};

pub trait DatabasePort: Send + Sync {
    fn get_machines(&self) -> Result<Vec<Machine>, String>;
    fn add_machine(&self, machine: Machine) -> Result<(), String>;
    fn delete_machine(&self, id: &str) -> Result<(), String>;
    fn update_machine(&self, machine: Machine) -> Result<(), String>;
    fn get_agent_profiles(&self, machine_id: &str) -> Result<Vec<AgentProfile>, String>;
    fn add_agent_profile(&self, profile: AgentProfile) -> Result<(), String>;
    fn delete_agent_profile(&self, id: &str) -> Result<(), String>;

    // Chat & History
    fn create_chat_session(&self, id: &str, agent_id: &str, title: &str) -> Result<(), String>;
    fn get_chat_sessions(&self, agent_id: &str) -> Result<Vec<ChatSession>, String>;
    fn add_chat_message(&self, id: &str, session_id: &str, sender: &str, content: &str) -> Result<(), String>;
    fn get_chat_messages(&self, session_id: &str) -> Result<Vec<ChatMessage>, String>;
    fn add_session_history(&self, id: &str, machine_id: &str, session_type: &str, title: &str, content: Option<&str>) -> Result<(), String>;
    fn get_session_history(&self, machine_id: &str) -> Result<Vec<SessionHistory>, String>;

    // Thread Sessions
    fn get_thread_sessions(&self, machine_id: &str) -> Result<Vec<ThreadSession>, String>;
    fn get_thread_sessions_for_thread(&self, thread_id: &str) -> Result<Vec<ThreadSession>, String>;
    fn add_thread_session(&self, thread: ThreadSession) -> Result<(), String>;
    fn update_thread_status(&self, id: &str, status: &str) -> Result<(), String>;
    fn delete_thread_session(&self, id: &str) -> Result<(), String>;

    // Agent configs (per machine, structured). Reads return the migrated,
    // typed records; writes accept a JSON-encoded string for forward-compat.
    fn get_agent_configs(&self, machine_id: &str) -> Result<Vec<AgentConfig>, String>;
    fn set_agent_configs(&self, machine_id: &str, agents_json: &str) -> Result<(), String>;

    // Working memory (per thread)
    fn upsert_working_memory_entry(
        &self,
        thread_id: &str,
        entry: WorkingMemoryEntry,
    ) -> Result<(), String>;
    fn get_working_memory(&self, thread_id: &str) -> Result<Vec<WorkingMemoryEntry>, String>;
    fn clear_working_memory(&self, thread_id: &str) -> Result<(), String>;

    // App session (key-value store for UI state)
    fn get_app_session(&self, key: &str) -> Result<Option<String>, String>;
    fn set_app_session(&self, key: &str, value: &str) -> Result<(), String>;
    fn delete_app_session(&self, key: &str) -> Result<(), String>;

    // Messages — the canonical conversation history
    fn get_messages(&self, thread_id: &str) -> Result<Vec<Message>, String>;
    fn append_message(&self, msg: &Message) -> Result<(), String>;
    fn delete_messages(&self, thread_id: &str) -> Result<(), String>;

    // Thread timestamp tracking for sidebar ordering
    fn update_thread_timestamp(&self, id: &str) -> Result<(), String>;

    // Persist the selected model for a thread session
    fn update_thread_model(&self, id: &str, model: &str) -> Result<(), String>;

    // Redesign Phase R1 Additions
    fn add_provider_instance(&self, provider: ProviderInstance) -> Result<(), String>;
    fn get_provider_instances(&self) -> Result<Vec<ProviderInstance>, String>;
    fn delete_provider_instance(&self, id: &str) -> Result<(), String>;

    fn add_project(&self, project: Project) -> Result<(), String>;
    fn get_projects(&self) -> Result<Vec<Project>, String>;
    fn update_project_status(&self, id: &str, status: &str) -> Result<(), String>;

    fn add_repository(&self, repo: Repository) -> Result<(), String>;
    fn get_repositories_for_project(&self, project_id: &str) -> Result<Vec<Repository>, String>;

    fn add_feature(&self, feature: Feature) -> Result<(), String>;
    fn get_active_features(&self, project_id: &str) -> Result<Vec<Feature>, String>;

    fn get_project_settings(&self, project_id: &str) -> Result<Option<ProjectSettings>, String>;
    fn save_project_settings(&self, settings: ProjectSettings) -> Result<(), String>;

    fn update_project(&self, project: Project) -> Result<(), String>;
    fn delete_project(&self, id: &str) -> Result<(), String>;
    fn delete_repositories_for_project(&self, project_id: &str) -> Result<(), String>;
}
