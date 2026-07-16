# gac Architecture Documentation

> **AI-powered commit message generator** using llama-swap (llama.cpp proxy) with intelligent diff strategies.

---

## Overview

`gac` is a Rust CLI tool that generates conventional commit messages from staged git changes. It uses a three-tier diff strategy to handle diffs of any size within the LLM context window:

| Strategy | Trigger | Approach |
|----------|---------|----------|
| **Direct** | Diff fits in context | Send full diff directly to LLM |
| **Summarize** | ≤ 20 files, too large for direct | Summarize each file via separate LLM calls, combine summaries |
| **StatOnly** | > 20 files | Use `git diff --stat` + top-N priority file diffs |

The config cascade: **defaults → user config (`~/.config/gac/config.toml`) → project config (`.gac.toml`)**. CLI only overrides `--model`.

---

## Functional Areas (Clusters)

Based on GitNexus code analysis, the codebase organizes into **14 functional clusters** with varying cohesion:

| Cluster | Symbols | Cohesion | Key Files | Description |
|---------|---------|----------|-----------|-------------|
| **Cluster_3** (Diff Strategy) | 12 | 0.77 | `diff.rs` | Strategy selection, priority scoring, context building |
| **Cluster_7** (CLI Entry) | 10 | 0.45 | `main.rs` | CLI parsing, orchestration, approval flow |
| **Cluster_0** (Config) | 7 | 0.75 | `config.rs` | Config loading, cascading, validation, encryption |
| **Cluster_2** (Diff Parsing) | 7 | 0.86 | `diff.rs` | Git diff parsing, file scoring, path extraction |
| **Cluster_8** (Scope Detection) | 7 | 0.95 | `git.rs` | Scope matching against staged files |
| **Cluster_11** (LLM Streaming) | 7 | 0.67 | `llamaswap.rs` | llama-swap API client (chat, tokenize, summarize, VRAM) |
| **Cluster_12** (Prompt Templates) | 4 | 0.67 | `prompt.rs` | Askama templates for commit, summary, retry prompts |
| **Cluster_15** (Validation) | 4 | 0.86 | `validate.rs` | Conventional commit validation + auto-fix |
| **Cluster_6** (Generation Stats) | 5 | 0.80 | `stats.rs` | Token/timing/VRAM display with visual bars |
| **Cluster_1** (Git Ops) | 3 | 0.80 | `git.rs` | Staged files, diff execution, commit execution |
| **Cluster_13** (Spinner UI) | 3 | 0.80 | `spinner.rs` | Progress spinners/bars via indicatif |
| **Cluster_14** (Crypto) | 2 | 1.00 | `crypto.rs` | AES-256-GCM + Argon2id API key encryption |
| **Build_summary_context** | 3 | 0.80 | `diff.rs` | Summary context builder |
| **Build_stat_context** | 3 | 0.80 | `diff.rs` | Stat-only context builder |

---

## Key Execution Flows (Processes)

GitNexus identifies **33 execution flows**. The top 5 by step count:

### 1. `Select_strategy → FileDiff` (Cross-community, 4 steps)
**Entry:** `diff.rs:select_strategy` → `diff.rs:parse_diff` → `diff.rs:build_direct_context` → `diff.rs:Strategy::Direct`
**Communities:** Cluster_3 (Diff Strategy) ↔ Cluster_2 (Diff Parsing)
**Purpose:** Determines if full diff fits in context window; falls back to Summarize/StatOnly.

### 2. `Select_strategy → Score_file` (Cross-community, 4 steps)
**Entry:** `diff.rs:select_strategy` → scores each file via `score_file` → decides strategy
**Communities:** Cluster_3 ↔ Cluster_2
**Purpose:** Priority scoring drives which files get full diffs in StatOnly mode.

### 3. `Parse_diff_single_file → FileDiff` (Intra-community, 4 steps)
**Entry:** `diff.rs:parse_diff_single_file` → `extract_path` → `flush` → `FileDiff`
**Community:** Cluster_2 (Diff Parsing)
**Purpose:** Parses individual file diff from raw git output.

### 4. `Parse_diff_multiple_files → FileDiff` (Intra-community, 4 steps)
**Entry:** `diff.rs:parse_diff_multiple_files` → iterates `parse_diff_single_file` → sorts by priority
**Community:** Cluster_2
**Purpose:** Splits multi-file diff, scores each file, returns priority-sorted list.

### 5. `Main → To_filter` / `Main → Fmt` / `Main → CliFormatter` (Cross-community, 3 steps)
**Entry:** `main.rs:main` → config loading → git operations → diff strategy → prompt → LLM → validation → approval → commit
**Communities:** Cluster_7 (CLI) ↔ Cluster_1 (Git Ops) ↔ Cluster_0 (Config) ↔ Cluster_3 (Diff) ↔ Cluster_11 (LLM) ↔ Cluster_15 (Validate)

---

## Mermaid Architecture Diagram

```mermaid
graph TB
    %% CLI Entry Point
    subgraph CLI["CLI Entry (main.rs)"]
        Main[main()]
        CliParse[CLI Parsing\nclap]
        LogInit[Logging Init\ntracing]
        ApprovalDialog[Approval Dialog\ndialoguer]
    end

    %% Configuration Layer
    subgraph Config["Configuration (config.rs)"]
        ConfigLoad[Config::load()\nCascade: default → user → project]
        ConfigValidate[Config::validate()]
        Crypto[Crypto Module\nAES-256-GCM + Argon2id]
        Scopes[Scope Definitions\nGlob patterns]
    end

    %% Git Operations
    subgraph Git["Git Operations (git.rs)"]
        GetStaged[get_staged_files()]
        GetStatDiff[get_staged_stat_and_diff()\n--stat + full diff]
        DetectScopes[detect_scopes()\nGlob matching]
        Excludes[get_excluded_files()\n:(exclude) patterns]
        GitCommit[commit()\ngit commit -m]
    end

    %% Diff Processing
    subgraph Diff["Diff Processing (diff.rs)"]
        ParseDiff[parse_diff()\nSplit by file]
        ScoreFile[score_file()\nPriority 0-100]
        SelectStrategy[select_strategy()\nToken budget check]
        
        subgraph Strategies["Strategy Implementations"]
            Direct[Direct\nFull diff in context]
            Summarize[Summarize\nPer-file LLM summaries]
            StatOnly[StatOnly\n--stat + top-N files]
        end
        
        BuildDirect[build_direct_context()]
        BuildSummary[build_summary_context()]
        BuildStat[build_stat_context()]
    end

    %% LLM Client (llama-swap)
    subgraph LLM["llama-swap Client (llamaswap.rs)"]
        CreateClient[create_client()]
        GenerateStream[generate_streaming()\nSSE streaming]
        Summarize[summarize()\nPer-file summary]
        Tokenize[tokenize() / detokenize()]
        TokenCounts[token_counts()\nPrompt token budget]
        ModelProps[model_props()\nn_ctx budget]
        ApplyTemplate[apply_template()\nChat template]
        QueryVRAM[query_vram()\nGPU stats]
        RetryLogic[with_retry()\nExponential backoff]
    end

    %% Prompt Building
    subgraph Prompt["Prompt Templates (prompt.rs)"]
        CommitPrompt[build_commit_prompt()\nSystem + User]
        SummaryPrompt[build_file_summary_prompt()]
        RetryPrompt[build_retry_prompt()\nValidation feedback]
        Templates[Askama Templates\ncommit_system.md, etc.]
    end

    %% Validation & Stats
    subgraph Validate["Validation (validate.rs)"]
        ValidateConv[validate_conventional_commit()\nRegex + rules]
        AutoFix[try_fix_commit_message()\nCapitalization, period]
        MaxRetries[MAX_VALIDATION_RETRIES = 2]
    end

    subgraph Stats["Generation Stats (stats.rs)"]
        GenStats[GenerationStats\nTokens, timing, VRAM]
        VRAMBar[VRAM Bar Visual\nUnicode blocks]
        PrintStats[print()\nStyled output]
    end

    %% Spinner UI
    subgraph Spinner["Progress UI (spinner.rs)"]
        MultiProgress[MultiProgress\nShared draw target]
        StepSpinner[Step Spinner]
        SummarizeBar[Summarize Progress Bar]
    end

    %% Flow Connections
    Main --> CliParse
    Main --> LogInit
    Main --> ConfigLoad
    ConfigLoad --> Crypto
    ConfigLoad --> ConfigValidate
    ConfigLoad --> Scopes
    
    Main --> GetStaged
    GetStaged --> GetStatDiff
    GetStatDiff --> Excludes
    GetStatDiff --> DetectScopes
    DetectScopes --> Scopes
    
    GetStatDiff --> ParseDiff
    ParseDiff --> ScoreFile
    ScoreFile --> SelectStrategy
    
    SelectStrategy -->|fits| Direct
    SelectStrategy -->|≤20 files| Summarize
    SelectStrategy -->|>20 files| StatOnly
    
    Direct --> BuildDirect
    Summarize --> BuildSummary
    StatOnly --> BuildStat
    
    BuildDirect --> CommitPrompt
    BuildSummary --> CommitPrompt
    BuildStat --> CommitPrompt
    
    CommitPrompt --> GenerateStream
    Summarize --> SummaryPrompt
    SummaryPrompt --> Summarize
    Summarize --> GenerateStream
    
    GenerateStream --> TokenCounts
    TokenCounts --> ModelProps
    GenerateStream --> QueryVRAM
    GenerateStream --> RetryLogic
    ApplyTemplate --> GenerateStream
    
    GenerateStream --> GenStats
    GenStats --> PrintStats
    GenStats --> VRAMBar
    
    GenerateStream --> ValidateConv
    ValidateConv -->|fail ≤2| RetryPrompt
    RetryPrompt --> GenerateStream
    ValidateConv -->|fail 3rd| AutoFix
    AutoFix --> ValidateConv
    
    ValidateConv -->|pass| ApprovalDialog
    ApprovalDialog -->|Commit| GitCommit
    ApprovalDialog -->|Edit| ApprovalDialog
    ApprovalDialog -->|Abort| Main
    
    %% Styling
    classDef entry fill:#e1f5fe,stroke:#01579b
    classDef config fill:#f3e5f5,stroke:#4a148c
    classDef git fill:#e8f5e9,stroke:#1b5e20
    classDef diff fill:#fff3e0,stroke:#e65100
    classDef llm fill:#fce4ec,stroke:#880e4f
    classDef prompt fill:#ede7f6,stroke:#311b92
    classDef validate fill:#f1f8e9,stroke:#33691e
    classDef stats fill:#fff8e1,stroke:#f57f17
    classDef ui fill:#f5f5f5,stroke:#424242
    
    class Main,CliParse,LogInit,ApprovalDialog entry
    class ConfigLoad,ConfigValidate,Crypto,Scopes config
    class GetStaged,GetStatDiff,DetectScopes,Excludes,GitCommit git
    class ParseDiff,ScoreFile,SelectStrategy,Direct,Summarize,StatOnly,BuildDirect,BuildSummary,BuildStat diff
    class CreateClient,GenerateStream,Summarize,Tokenize,TokenCounts,ModelProps,ApplyTemplate,QueryVRAM,RetryLogic llm
    class CommitPrompt,SummaryPrompt,RetryPrompt,Templates prompt
    class ValidateConv,AutoFix,MaxRetries validate
    class GenStats,VRAMBar,PrintStats stats
    class MultiProgress,StepSpinner,SummarizeBar ui
```

---

## Data Flow Summary

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│  git diff   │────▶│  parse_diff │────▶│ FileDiff[]  │
│  --cached   │     │  (priority  │     │ (sorted by  │
└─────────────┘     │   scored)   │     │  priority)  │
                    └─────────────┘     └──────┬──────┘
                                               │
                    ┌──────────────────────────┼──────────────────────────┐
                    ▼                          ▼                          ▼
            ┌───────────────┐           ┌───────────────┐           ┌───────────────┐
            │   Direct      │           │   Summarize   │           │   StatOnly    │
            │  (tokens <    │           │  (files ≤ 20) │           │  (files > 20) │
            │   n_ctx)      │           │               │           │               │
            └───────┬───────┘           └───────┬───────┘           └───────┬───────┘
                    │                           │                           │
                    ▼                           ▼                           ▼
            ┌───────────────┐           ┌───────────────┐           ┌───────────────┐
            │build_direct_  │           │ For each file:│           │build_stat_    │
            │context()      │           │  summarize()  │           │context()      │
            └───────┬───────┘           └───────┬───────┘           └───────┬───────┘
                    │                           │                           │
                    └───────────────────────────┼───────────────────────────┘
                                                ▼
                                    ┌───────────────────────┐
                                    │  build_commit_prompt  │
                                    │  (system + user)      │
                                    └───────────┬───────────┘
                                                │
                                                ▼
                                    ┌───────────────────────┐
                                    │  generate_streaming   │
                                    │  (llama-swap SSE)     │
                                    └───────────┬───────────┘
                                                │
                                                ▼
                                    ┌───────────────────────┐
                                    │  validate_conventional│
                                    │  _commit() + retries  │
                                    └───────────┬───────────┘
                                                │
                                                ▼
                                    ┌───────────────────────┐
                                    │  approval_dialog()    │
                                    │  (commit/edit/abort)  │
                                    └───────────┬───────────┘
                                                │
                                                ▼
                                    ┌───────────────────────┐
                                    │   git commit -m       │
                                    └───────────────────────┘
```

---

## Key Design Decisions

### 1. Three-Tier Diff Strategy
- **Direct** path avoids extra LLM calls when diff fits
- **Summarize** uses parallel-ish per-file calls (sequential with shared progress bar)
- **StatOnly** caps at 20 files to avoid token explosion

### 2. Priority Scoring Heuristics
- Source files in `src/`/`lib/` = 90
- Source files elsewhere = 75
- Test files = 40
- Config/docs = 20
- Lock files = 0 (excluded)

### 3. Config Cascade (No `[model]` Nesting)
- Flat TOML: `endpoint`, `model`, `max_completion_tokens` at root
- Project config **replaces** user scopes entirely (no merge)
- CLI only overrides `--model`

### 4. Validation with Retry + Auto-Fix
- Regex validates conventional commit format
- Up to 2 retries with error context fed back to LLM
- Auto-fix handles capitalization, trailing period
- Final fallback: user edits in `$EDITOR` (default `nvim`)

### 5. llama-swap API (Not Ollama)
- `/v1/chat/completions` with SSE streaming
- `/tokenize`, `/detokenize`, `/props` for budget management
- Retry with exponential backoff (max 3, base 500ms)

### 6. Encrypted API Key Storage
- AES-256-GCM + Argon2id (64MiB, 3 iterations)
- Password from `GAC_ENCRYPTION_KEY` env or default
- Stored in user config as `api_key_encrypted`

---

## Testing

```bash
cargo test                    # All tests
cargo test diff               # Diff module tests
cargo test diff::tests::test_parse_diff_single_file  # Single test
cargo test -- --nocapture     # With stdout
```

Key test modules:
- `diff.rs` — scoring, parsing, context builders, strategy selection
- `git.rs` — scope detection, exclude patterns
- `prompt.rs` — template rendering, retry prompts
- `validate.rs` — conventional commit validation, auto-fix
- `llamaswap.rs` — tokenization, chunk parsing, message building

---

## Extending the Architecture

| Extension Point | Location | Notes |
|-----------------|----------|-------|
| New diff strategy | `diff.rs::Strategy` enum | Add variant + handler in `main.rs` |
| New prompt template | `templates/*.md` + `prompt.rs` | Askama compiles at build time |
| New validation rule | `validate.rs` | Add to regex or `try_fix_commit_message` |
| New LLM endpoint | `llamaswap.rs` | Follow existing retry/streaming patterns |
| New config field | `config.rs::FileConfig` + `Config` | Add to cascade in `apply_file` |

---

*Generated from GitNexus code analysis (379 symbols, 681 relationships, 33 execution flows)*