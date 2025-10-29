 ---
  🎯 Core Concept: What is Serena?

  Serena is a language-server-powered coding agent toolkit
  that bridges the gap between LLMs and IDE-level code
  intelligence. Instead of treating code as plain text, Serena
   enables LLMs to work with semantic code structures -
  functions, classes, methods, and their relationships.

  The fundamental innovation: LLMs can find, navigate, and
  edit code at the symbol level rather than using regex and
  line numbers.

  ---
  🏗️ High-Level Architecture

  ┌─────────────────────────────────────────────────────────┐
  │                    AI Client Layer                       │
  │          (Claude Code, Claude Desktop, IDEs)            │
  └────────────────────┬────────────────────────────────────┘
         P0+r\P0+r\              │ MCP Protocol (stdio/HTTP)
  ┌────────────────────▼────────────────────────────────────┐
  │                  SerenaAgent (Core)                      │
  │  ┌─────────────┬──────────────┬─────────────────────┐  │
  │  │ Tool System │ Config/Modes │ Project Management  │  │
  │  └─────────────┴──────────────┴─────────────────────┘  │
  └────────────────────┬────────────────────────────────────┘
                       │
  ┌────────────────────▼────────────────────────────────────┐
  │           LanguageServerManager                          │
  │  Orchestrates multiple language servers per project     │
  └────────────────────┬────────────────────────────────────┘
                       │
  ┌────────────────────▼────────────────────────────────────┐
  │              SolidLanguageServer(s)                      │
  │  Python │ TypeScript │ Rust │ Go │ Java │ [15+ more]   │
  └────────────────────┬────────────────────────────────────┘
                       │ LSP Protocol
  ┌────────────────────▼────────────────────────────────────┐
  │            Actual Language Servers                       │
  │  Pyright │ tsserver │ rust-analyzer │ gopls │ ...       │
  └──────────────────────────────────────────────────────────┘

  ---
  🔧 Core Components Deep Dive

  1. SerenaAgent (src/serena/agent.py)

  The central orchestrator that manages everything:

  Initialization Flow:
  SerenaAgent.__init__()
    ├── Load SerenaConfig from ~/.serena/serena_config.yml
    ├── Initialize Tool Registry (25+ tools)
    ├── Apply Context (desktop-app, ide-assistant, agent,
  codex)
    ├── Apply Modes (planning, editing, interactive, one-shot)
    ├── Activate Project (if --project specified)
    │   ├── Load .serena/project.yml
    │   ├── Start LanguageServerManager
    │   └── Index source files
    └── Start Dashboard (optional web UI on :24282)

  Key Responsibilities:
  - Tool Management: Registry and execution of all available
  tools
  - Context/Mode Application: Filters tools and modifies
  prompts based on environment
  - Project Lifecycle: Activates/deactivates projects with
  proper LS cleanup
  - Async Execution: ThreadPoolExecutor for tool execution
  with timeout handling
  - Monitoring: Web dashboard and GUI log viewer

  2. SolidLanguageServer (src/solidlsp/ls.py)

  Synchronous wrapper around asynchronous LSP implementations:

  Architecture:
  # Base class with 20+ language-specific subclasses
  class SolidLanguageServer(ABC):
      def request_document_symbols(file_path) ->
  List[UnifiedSymbolInformation]
      def request_references(file_path, line, col) ->
  List[Location]
      def request_definition(file_path, line, col) -> Location
      def rename_symbol(file_path, line, col, new_name) ->
  WorkspaceEdit

      # Caching layer
      def _load_symbols_from_cache() -> Optional[List[Symbol]]
      def _save_symbols_to_cache(symbols)

  Language Server Implementations:
  - Python: Pyright (fast, type-aware)
  - TypeScript/JavaScript: typescript-language-server
  - Rust: rust-analyzer (macro expansion, trait resolution)
  - Go: gopls (workspace awareness)
  - Java: Eclipse JDT.LS (Maven/Gradle support)
  - C/C++: clangd (compilation database)
  - C#: OmniSharp or omnisharp-roslyn
  - Plus: Scala (Metals), Haskell (HLS), Elixir, Erlang,
  Clojure, Ruby, PHP, Swift, Kotlin, Dart, Lua, Bash,
  Terraform, Fortran, Julia, and more

  How It Works:
  1. Auto-Download: If LS binary not found, downloads to
  ~/.serena/ls_resources/
  2. Process Management: Spawns LS as subprocess, communicates
   via stdin/stdout
  3. Initialization: Sends LSP initialize with workspace root
  4. Symbol Caching: Persists document symbols to
  .serena/cache/{file_hash}.json
  5. File Tracking: Manages open/close notifications, content
  versions

  3. Symbol System (src/serena/symbol.py)

  The core abstraction for semantic code understanding:

  class LanguageServerSymbol:
      name: str                    # "calculate_total"
      kind: SymbolKind            # Function, Class, Method,
  Variable, etc.
      location: Location          # file_path + line/col range
      body: str | None            # Full source code of symbol
      children: List[Symbol]      # Nested symbols (methods in
   class, etc.)

      # Navigation
      def get_name_path() -> str  # "MyClass/my_method"
      def find(pattern) -> List[Symbol]  # Search hierarchy
      def iter_ancestors() -> Iterator[Symbol]

  Symbol Hierarchy Example:
  # For a file like:
  class Calculator:
      def add(self, a, b):
          return a + b

      def subtract(self, a, b):
          return a - b

  # Serena creates:
  LanguageServerSymbol(
      name="Calculator",
      kind=SymbolKind.Class,
      children=[
          LanguageServerSymbol(name="add",
  kind=SymbolKind.Method),
          LanguageServerSymbol(name="subtract",
  kind=SymbolKind.Method)
      ]
  )

  LanguageServerSymbolRetriever:
  High-level facade for symbol operations:
  retriever = LanguageServerSymbolRetriever(ls_manager)

  # Find by name across project
  symbols = retriever.find_by_name("Calculator",
  file_path="src/")

  # Find references to a symbol
  refs = retriever.find_referencing_symbols(symbol)

  # Navigate to definition
  definition = retriever.find_definition(file, line, col)

  4. Tool System (src/serena/tools/)

  Organized into specialized modules:

  Symbol Tools (symbol_tools.py) - The Power Tools

  - FindSymbolTool: Search by name/pattern
  {
    "name_path": "Calculator/add",
    "relative_path": "src/math.py"
  }
  - FindReferencingSymbolsTool: Find all usages
  {
    "symbol_name_path": "calculate_total",
    "relative_path": "billing.py"
  }
  - GetSymbolsOverviewTool: File outline
  {"relative_path": "models.py"}
  → Returns: "Class User, Class Product, Function
  validate_email"
  - RenameSymbolTool: Refactor across codebase
  {
    "old_name_path": "getUserData",
    "new_name": "get_user_data",
    "relative_path": "api.py"
  }
  - ReplaceSymbolBodyTool: Replace implementation
  {
    "name_path": "calculate_total",
    "relative_path": "billing.py",
    "new_body": "def calculate_total(items):\n    return
  sum(items) * 1.1"
  }
  - InsertBeforeSymbolTool/InsertAfterSymbolTool: Positional
  insertion
  {
    "name_path": "MyClass",
    "relative_path": "models.py",
    "text_to_insert": "@dataclass\n"
  }

  File Tools (file_tools.py) - Traditional Operations

  - ReadFileTool, CreateTextFileTool, WriteFileTool
  - SearchForPatternTool (ripgrep wrapper)
  - ReplaceRegexTool (find/replace)
  - ListDirTool, FindFileTool

  Memory Tools (memory_tools.py) - Knowledge Persistence

  - WriteMemoryTool: Save insights to .serena/memories/
  - ReadMemoryTool: Retrieve saved knowledge
  - ListMemoriesTool, DeleteMemoryTool

  Example Memory Usage:
  # .serena/memories/architecture.md
  # Project Architecture

  ## Database
  - Uses PostgreSQL with SQLAlchemy ORM
  - Connection via POSTGRES_URI env var
  - Migration tool: Alembic

  ## Testing
  - Run: pytest test/
  - Coverage: pytest --cov=src/

  ## Build
  - uv for dependency management
  - Build: uv build
  - Run: uv run python -m myapp

  Config Tools (config_tools.py) - Runtime Control

  - ActivateProjectTool: Switch projects mid-session
  - SwitchModesTool: Change operational modes
  - GetCurrentConfigTool: Introspection

  Workflow Tools (workflow_tools.py) - Guided Processes

  - OnboardingTool: Initial project exploration
  - PrepareForNewConversationTool: Context handoff
  - CheckOnboardingPerformedTool: State tracking

  ---
  🔄 Complete Request Flow Example

  Scenario: User asks "Rename the getUserData function to
  get_user_data"

  1. Client (Claude Code) sends MCP request:
     {
       "method": "tools/call",
       "params": {
         "name": "rename_symbol",
         "arguments": {
           "old_name_path": "getUserData",
           "new_name": "get_user_data",
           "relative_path": "src/api.py"
         }
       }
     }

  2. MCP Server (mcp.py) receives request:
     - Validates schema
     - Looks up RenameSymbolTool in SerenaAgent.tools

  3. RenameSymbolTool.apply() executes:
     a. Find symbol location:
        -
  LanguageServerSymbolRetriever.find_by_name("getUserData",
  "src/api.py")
        - LanguageServerManager determines language (Python)
        - Returns PythonLanguageServer instance

     b.
  PythonLanguageServer.request_document_symbols("src/api.py"):
        - Check cache: .serena/cache/api_py_abc123.json
        - Cache hit → Load symbols
        - Find symbol "getUserData" at line 45, col 4

     c. PythonLanguageServer.rename_symbol(file, 45, 4,
  "get_user_data"):
        - Sends LSP textDocument/rename request to Pyright
        - Pyright analyzes all references across project
        - Returns WorkspaceEdit with all file changes

     d. Apply changes:
        - RenameSymbolTool iterates WorkspaceEdit
        - Modifies files: src/api.py, src/tests/test_api.py,
  src/main.py
        - Notifies language server of file changes

  4. Return result to client:
     {
       "content": [
         {
           "type": "text",
           "text": "Renamed getUserData to get_user_data
  across 3 files"
         }
       ]
     }

  ---
  ⚙️ Configuration & Customization

  Configuration Hierarchy (highest to lowest priority):

  1. CLI arguments: --context agent --mode planning
  2. Project config: .serena/project.yml
  3. User config: ~/.serena/serena_config.yml
  4. Defaults: Built-in contexts/modes

  Contexts - Define the Environment

  desktop-app (default):
  name: desktop-app
  description: "Standalone usage with Claude Desktop"
  prompt: |
    You are Serena, an AI coding assistant with full file
  system access.
    Use symbolic tools for precise code editing.
  included_optional_tools: [execute_shell_command]
  excluded_tools: []

  ide-assistant:
  name: ide-assistant
  description: "IDE extension complement (Cursor, Windsurf)"
  prompt: |
    You are running inside an IDE that has its own shell.
    Focus on symbolic operations. Do not execute shell
  commands.
  excluded_tools: [execute_shell_command]

  agent:
  name: agent
  description: "Agent framework integration (Agno, custom)"
  prompt: |
    You are an autonomous coding agent. Complete tasks fully.
    Use onboarding for new projects. Write memories for future
   sessions.
  included_optional_tools: [think_about_task_adherence]

  Modes - Operational Patterns

  editing:
  name: editing
  description: "Focus on making code changes"
  excluded_tools: [think_about_task_adherence]  # Less
  metacognition
  prompt: |
    Editing mode: Make precise code modifications.
    Use replace_symbol_body over regex replacement.

  planning:
  name: planning
  description: "Analysis before implementation"
  prompt: |
    Planning mode: Think before you code.
    1. Use get_symbols_overview to understand structure
    2. Use find_symbol to locate relevant code
    3. Plan the changes
    4. Only then modify code

  one-shot:
  name: one-shot
  description: "Complete task in single response"
  prompt: |
    One-shot mode: Provide complete solution immediately.
    Do not ask for clarification unless absolutely necessary.

  Combining Modes:
  serena start-mcp-server --mode planning --mode interactive
  # Both prompts are concatenated, tools from both are merged

  ---
  📊 MCP Server Implementation

  Protocol Flow:

  # mcp.py
  class SerenaMCPFactory:
      @staticmethod
      def create_mcp_server(agent: SerenaAgent,
  oai_compatible: bool):
          # Initialize FastMCP server
          mcp = FastMCP("Serena")

          # Register all tools
          for tool in agent.get_available_tools():
              mcp.tool(
                  name=tool.name,
                  description=tool.description,
                  parameters=tool.get_input_schema()
              )(create_tool_wrapper(tool))

          # System prompt
          @mcp.prompt()
          def system_prompt():
              return
  agent.prompt_factory.create_system_prompt()

          return mcp

  Tool Schema Conversion:

  Serena tools define schemas, MCP converts them:
  # Tool definition
  class FindSymbolTool(Tool):
      input_schema = {
          "type": "object",
          "properties": {
              "name_path": {"type": "string", "description":
  "Symbol name"},
              "relative_path": {"type": "string",
  "description": "File path"}
          },
          "required": ["name_path"]
      }

  # MCP exposes as:
  {
    "name": "find_symbol",
    "description": "Find a symbol by name in the codebase",
    "inputSchema": {
      "type": "object",
      "properties": {
        "name_path": {"type": "string"},
        "relative_path": {"type": "string"}
      },
      "required": ["name_path"]
    }
  }

  ---
  🚀 Performance Optimizations

  1. Symbol Caching:
    - First request: Parse file with LSP (~100-500ms)
    - Cached: Load from disk (~5-10ms)
    - Cache key: {file_path}_{content_hash}.json
  2. Lazy Language Server Startup:
    - LS starts only when first file of that language is
  accessed
    - Multiple LSs start in parallel threads
  3. Indexed Projects:
  serena project index /path/to/project
  # Pre-computes all symbols, stores in .serena/cache/
  # Subsequent sessions: instant symbol lookup
  4. Efficient Symbol Search:
    - Hierarchical tree traversal (not flat iteration)
    - Name path matching with early termination
    - Regex compilation cached
  5. Content Hashing:
    - Unchanged files skip re-parsing
    - Hash stored in cache filename

  ---
  🧠 Memory & Knowledge System

  Purpose: Persistent project knowledge across sessions

  Storage: .serena/memories/*.md (Markdown files)

  Workflow:
  Session 1 (Onboarding):
    User: "Help me understand this project"
    LLM:
      1. get_symbols_overview on key files
      2. read_file on README, tests
      3. execute_shell_command "cat pyproject.toml"
      4. write_memory("architecture", "# Architecture\n...")
      5. write_memory("testing_guide", "# Tests\nRun:
  pytest...")

  Session 2 (Later):
    User: "Add a new feature"
    LLM:
      1. list_memories → ["architecture", "testing_guide"]
      2. read_memory("architecture") → Loads context
      3. Implements feature following established patterns
      4. Runs tests per testing_guide

  Example Memories:
  - architecture.md: System design, key components
  - testing_guide.md: How to run tests, CI setup
  - build_commands.md: Build/deploy instructions
  - conventions.md: Code style, naming patterns
  - dependencies.md: Key libraries, versions

  ---
  🎨 Dashboard & Monitoring

  Web Dashboard (http://localhost:24282/dashboard/):
  ┌─────────────────────────────────────────┐
  │  Serena Dashboard                       │
  ├─────────────────────────────────────────┤
  │  Active Tools:                          │
  │    - find_symbol (15 calls, 2.5k tokens)│
  │    - replace_symbol_body (8 calls)      │
  │    - read_file (23 calls)               │
  │                                         │
  │  Real-time Logs:                        │
  │    [12:34:56] Tool: find_symbol         │
  │    [12:34:57] Result: Found 3 symbols   │
  │    [12:35:02] Tool: replace_symbol_body │
  │                                         │
  │  [Shutdown Server]                      │
  └─────────────────────────────────────────┘

  Features:
  - Real-time log streaming (WebSocket)
  - Tool usage statistics with token counts
  - Process management (kill orphaned servers)
  - Filterable log viewer

  ---
  🔑 Key Architectural Insights

  Why Symbol-Based Editing is Powerful:

  Traditional Text-Based:
  # LLM must:
  1. Read entire file
  2. Count lines to find target
  3. Craft regex to match function
  4. Hope indentation is correct
  5. Replace with string manipulation

  # Fragile, error-prone, slow

  Serena's Symbol-Based:
  # LLM does:
  1. find_symbol("calculate_total") → Symbol at line 45
  2. replace_symbol_body(symbol, new_code)

  # Precise, fast, language-aware

  Multi-Language Support via LSP:

  - One interface: All languages expose same symbol API
  - Language-specific intelligence: Each LS understands its
  language's nuances
  - Zero Serena code changes: Adding new language = subclass
  SolidLanguageServer

  Extensibility Patterns:

  1. New Tool: Subclass Tool, implement apply(),
  auto-registered
  2. New Language: Subclass SolidLanguageServer, define LS
  command
  3. New Context: Create YAML in resources/config/contexts/
  4. New Mode: Create YAML in resources/config/modes/

  ---
  🎯 Use Cases & Workflows

  1. Refactoring:

  find_symbol("OldClass") → Understand usage
  find_referencing_symbols("OldClass") → Find all references
  rename_symbol("OldClass", "NewClass") → Refactor safely

  2. Feature Addition:

  get_symbols_overview("models.py") → Understand structure
  find_symbol("BaseModel") → Find parent class
  insert_after_symbol("BaseModel", new_class_code) → Add new
  model

  3. Bug Fixing:

  search_for_pattern("error_pattern") → Locate issue
  find_symbol("buggy_function") → Get full context
  replace_symbol_body("buggy_function", fixed_code) → Fix

  4. Codebase Exploration:

  onboarding_tool() → Guided exploration
  write_memory("findings") → Document learnings
  prepare_for_new_conversation() → Context for next session

  ---
  This architecture makes Serena uniquely powerful because:

  1. Semantic Understanding: Works with code structures, not
  text
  2. Language Agnostic: 20+ languages via LSP
  3. IDE-Quality Intelligence: Same tech as VS Code, JetBrains
  4. Flexible Integration: MCP protocol works anywhere
  5. Persistent Knowledge: Memories survive sessions
  6. Scalable: Caching + indexing handle large codebases
  7. Customizable: Contexts + modes adapt to any workflow

  The core innovation: Giving LLMs the same code navigation
  tools humans use in IDEs, enabling them to think in
  functions, classes, and modules rather than regex and line
  numbers.

