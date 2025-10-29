use super::register_stub;
use crate::tool::ToolRegistry;

pub fn register(registry: &mut ToolRegistry) {
    register_stub(registry, "find_symbol", "Stubbed symbol lookup");
    register_stub(
        registry,
        "find_referencing_symbols",
        "Stubbed symbol reference search",
    );
    register_stub(registry, "get_symbols_overview", "Stubbed symbol overview");
    register_stub(registry, "rename_symbol", "Stubbed symbol rename");
    register_stub(
        registry,
        "replace_symbol_body",
        "Stubbed symbol body replacement",
    );
}
