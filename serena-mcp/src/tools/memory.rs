use super::register_stub;
use crate::tool::ToolRegistry;

pub fn register(registry: &mut ToolRegistry) {
    register_stub(
        registry,
        "write_memory",
        "Stubbed persistent memory storage",
    );
    register_stub(
        registry,
        "read_memory",
        "Stubbed persistent memory retrieval",
    );
    register_stub(registry, "list_memories", "Stubbed memory listing");
    register_stub(registry, "delete_memory", "Stubbed memory deletion");
}
