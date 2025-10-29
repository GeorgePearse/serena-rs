use super::register_stub;
use crate::tool::ToolRegistry;

pub fn register(registry: &mut ToolRegistry) {
    register_stub(registry, "onboarding_tool", "Stubbed onboarding workflow");
    register_stub(
        registry,
        "prepare_for_new_conversation",
        "Stubbed conversation hand-off",
    );
    register_stub(
        registry,
        "check_onboarding_performed",
        "Stubbed onboarding status check",
    );
}
