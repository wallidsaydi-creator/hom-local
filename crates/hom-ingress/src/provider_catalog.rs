// Generated/adapted from Open Interpreter 0.2.168 provider catalog seed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Local,
    ApiKey,
    OAuth,
    Custom,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireMode {
    OllamaNative,
    OpenAiChatCompletions,
    OpenAiResponses,
    AnthropicMessages,
    GoogleGemini,
    CodexOAuthRuntime,
}
#[derive(Debug, Clone, Copy)]
pub struct ModelSeed {
    pub id: &'static str,
    pub name: &'static str,
}
pub static INTERPRETER_MODELS: &[ModelSeed] = &[
    ModelSeed {
        id: "interpreter-smart",
        name: "Interpreter Smart",
    },
    ModelSeed {
        id: "interpreter-fast",
        name: "Interpreter Fast",
    },
];
pub static ANTHROPIC_MODELS: &[ModelSeed] = &[
    ModelSeed {
        id: "claude-3-haiku-20240307",
        name: "Claude Haiku 3",
    },
    ModelSeed {
        id: "claude-3-5-haiku-20241022",
        name: "Claude Haiku 3.5",
    },
    ModelSeed {
        id: "claude-3-5-haiku-latest",
        name: "Claude Haiku 3.5 (latest)",
    },
    ModelSeed {
        id: "claude-haiku-4-5-20251001",
        name: "Claude Haiku 4.5",
    },
    ModelSeed {
        id: "claude-haiku-4-5",
        name: "Claude Haiku 4.5 (latest)",
    },
    ModelSeed {
        id: "claude-3-opus-20240229",
        name: "Claude Opus 3",
    },
    ModelSeed {
        id: "claude-opus-4-20250514",
        name: "Claude Opus 4",
    },
    ModelSeed {
        id: "claude-opus-4-0",
        name: "Claude Opus 4 (latest)",
    },
    ModelSeed {
        id: "claude-opus-4-1-20250805",
        name: "Claude Opus 4.1",
    },
    ModelSeed {
        id: "claude-opus-4-1",
        name: "Claude Opus 4.1 (latest)",
    },
    ModelSeed {
        id: "claude-opus-4-5-20251101",
        name: "Claude Opus 4.5",
    },
    ModelSeed {
        id: "claude-opus-4-5",
        name: "Claude Opus 4.5 (latest)",
    },
    ModelSeed {
        id: "claude-opus-4-6",
        name: "Claude Opus 4.6",
    },
    ModelSeed {
        id: "claude-opus-4-7",
        name: "Claude Opus 4.7",
    },
    ModelSeed {
        id: "claude-3-sonnet-20240229",
        name: "Claude Sonnet 3",
    },
    ModelSeed {
        id: "claude-3-5-sonnet-20240620",
        name: "Claude Sonnet 3.5",
    },
    ModelSeed {
        id: "claude-3-5-sonnet-20241022",
        name: "Claude Sonnet 3.5 v2",
    },
    ModelSeed {
        id: "claude-3-7-sonnet-20250219",
        name: "Claude Sonnet 3.7",
    },
    ModelSeed {
        id: "claude-sonnet-4-20250514",
        name: "Claude Sonnet 4",
    },
    ModelSeed {
        id: "claude-sonnet-4-0",
        name: "Claude Sonnet 4 (latest)",
    },
    ModelSeed {
        id: "claude-sonnet-4-5-20250929",
        name: "Claude Sonnet 4.5",
    },
    ModelSeed {
        id: "claude-sonnet-4-5",
        name: "Claude Sonnet 4.5 (latest)",
    },
    ModelSeed {
        id: "claude-sonnet-4-6",
        name: "Claude Sonnet 4.6",
    },
];
pub static OPENAI_MODELS: &[ModelSeed] = &[
    ModelSeed {
        id: "gpt-4",
        name: "GPT-4",
    },
    ModelSeed {
        id: "gpt-4-turbo",
        name: "GPT-4 Turbo",
    },
    ModelSeed {
        id: "gpt-4.1",
        name: "GPT-4.1",
    },
    ModelSeed {
        id: "gpt-4.1-mini",
        name: "GPT-4.1 mini",
    },
    ModelSeed {
        id: "gpt-4.1-nano",
        name: "GPT-4.1 nano",
    },
    ModelSeed {
        id: "gpt-4o",
        name: "GPT-4o",
    },
    ModelSeed {
        id: "gpt-4o-2024-05-13",
        name: "GPT-4o (2024-05-13)",
    },
    ModelSeed {
        id: "gpt-4o-2024-08-06",
        name: "GPT-4o (2024-08-06)",
    },
    ModelSeed {
        id: "gpt-4o-2024-11-20",
        name: "GPT-4o (2024-11-20)",
    },
    ModelSeed {
        id: "gpt-4o-mini",
        name: "GPT-4o mini",
    },
    ModelSeed {
        id: "gpt-5",
        name: "GPT-5",
    },
    ModelSeed {
        id: "gpt-5-mini",
        name: "GPT-5 Mini",
    },
    ModelSeed {
        id: "gpt-5-nano",
        name: "GPT-5 Nano",
    },
    ModelSeed {
        id: "gpt-5-pro",
        name: "GPT-5 Pro",
    },
    ModelSeed {
        id: "gpt-5-codex",
        name: "GPT-5-Codex",
    },
    ModelSeed {
        id: "gpt-5.1",
        name: "GPT-5.1",
    },
    ModelSeed {
        id: "gpt-5.1-chat-latest",
        name: "GPT-5.1 Chat",
    },
    ModelSeed {
        id: "gpt-5.1-codex",
        name: "GPT-5.1 Codex",
    },
    ModelSeed {
        id: "gpt-5.1-codex-max",
        name: "GPT-5.1 Codex Max",
    },
    ModelSeed {
        id: "gpt-5.1-codex-mini",
        name: "GPT-5.1 Codex mini",
    },
    ModelSeed {
        id: "gpt-5.2",
        name: "GPT-5.2",
    },
    ModelSeed {
        id: "gpt-5.2-chat-latest",
        name: "GPT-5.2 Chat",
    },
    ModelSeed {
        id: "gpt-5.2-codex",
        name: "GPT-5.2 Codex",
    },
    ModelSeed {
        id: "gpt-5.2-pro",
        name: "GPT-5.2 Pro",
    },
    ModelSeed {
        id: "gpt-5.3-chat-latest",
        name: "GPT-5.3 Chat (latest)",
    },
    ModelSeed {
        id: "gpt-5.3-codex",
        name: "GPT-5.3 Codex",
    },
    ModelSeed {
        id: "gpt-5.3-codex-spark",
        name: "GPT-5.3 Codex Spark",
    },
    ModelSeed {
        id: "gpt-5.4",
        name: "GPT-5.4",
    },
    ModelSeed {
        id: "gpt-5.4-mini",
        name: "GPT-5.4 mini",
    },
    ModelSeed {
        id: "gpt-5.4-nano",
        name: "GPT-5.4 nano",
    },
    ModelSeed {
        id: "gpt-5.4-pro",
        name: "GPT-5.4 Pro",
    },
    ModelSeed {
        id: "gpt-5.5",
        name: "GPT-5.5",
    },
    ModelSeed {
        id: "gpt-5.5-pro",
        name: "GPT-5.5 Pro",
    },
    ModelSeed {
        id: "o1",
        name: "o1",
    },
    ModelSeed {
        id: "o1-pro",
        name: "o1-pro",
    },
    ModelSeed {
        id: "o3",
        name: "o3",
    },
    ModelSeed {
        id: "o3-deep-research",
        name: "o3-deep-research",
    },
    ModelSeed {
        id: "o3-mini",
        name: "o3-mini",
    },
    ModelSeed {
        id: "o3-pro",
        name: "o3-pro",
    },
    ModelSeed {
        id: "o4-mini",
        name: "o4-mini",
    },
    ModelSeed {
        id: "o4-mini-deep-research",
        name: "o4-mini-deep-research",
    },
];
pub static GROQ_MODELS: &[ModelSeed] = &[
    ModelSeed {
        id: "groq/compound",
        name: "Compound",
    },
    ModelSeed {
        id: "groq/compound-mini",
        name: "Compound Mini",
    },
    ModelSeed {
        id: "openai/gpt-oss-120b",
        name: "GPT OSS 120B",
    },
    ModelSeed {
        id: "openai/gpt-oss-20b",
        name: "GPT OSS 20B",
    },
    ModelSeed {
        id: "moonshotai/kimi-k2-instruct-0905",
        name: "Kimi K2 Instruct 0905",
    },
    ModelSeed {
        id: "llama-3.1-8b-instant",
        name: "Llama 3.1 8B Instant",
    },
    ModelSeed {
        id: "llama-3.3-70b-versatile",
        name: "Llama 3.3 70B Versatile",
    },
    ModelSeed {
        id: "meta-llama/llama-4-scout-17b-16e-instruct",
        name: "Llama 4 Scout 17B",
    },
    ModelSeed {
        id: "qwen/qwen3-32b",
        name: "Qwen3 32B",
    },
    ModelSeed {
        id: "openai/gpt-oss-safeguard-20b",
        name: "Safety GPT OSS 20B",
    },
];
pub static OPENROUTER_MODELS: &[ModelSeed] = &[
    ModelSeed {
        id: "anthropic/claude-3.5-haiku",
        name: "Claude Haiku 3.5",
    },
    ModelSeed {
        id: "anthropic/claude-haiku-4.5",
        name: "Claude Haiku 4.5",
    },
    ModelSeed {
        id: "anthropic/claude-opus-4",
        name: "Claude Opus 4",
    },
    ModelSeed {
        id: "anthropic/claude-opus-4.1",
        name: "Claude Opus 4.1",
    },
    ModelSeed {
        id: "anthropic/claude-opus-4.5",
        name: "Claude Opus 4.5",
    },
    ModelSeed {
        id: "anthropic/claude-opus-4.6",
        name: "Claude Opus 4.6",
    },
    ModelSeed {
        id: "anthropic/claude-opus-4.7",
        name: "Claude Opus 4.7",
    },
    ModelSeed {
        id: "anthropic/claude-3.7-sonnet",
        name: "Claude Sonnet 3.7",
    },
    ModelSeed {
        id: "anthropic/claude-sonnet-4",
        name: "Claude Sonnet 4",
    },
    ModelSeed {
        id: "anthropic/claude-sonnet-4.5",
        name: "Claude Sonnet 4.5",
    },
    ModelSeed {
        id: "anthropic/claude-sonnet-4.6",
        name: "Claude Sonnet 4.6",
    },
    ModelSeed {
        id: "mistralai/codestral-2508",
        name: "Codestral 2508",
    },
    ModelSeed {
        id: "deepseek/deepseek-v3.1-terminus",
        name: "DeepSeek V3.1 Terminus",
    },
    ModelSeed {
        id: "deepseek/deepseek-v3.1-terminus:exacto",
        name: "DeepSeek V3.1 Terminus (exacto)",
    },
    ModelSeed {
        id: "deepseek/deepseek-v3.2",
        name: "DeepSeek V3.2",
    },
    ModelSeed {
        id: "deepseek/deepseek-v3.2-speciale",
        name: "DeepSeek V3.2 Speciale",
    },
    ModelSeed {
        id: "deepseek/deepseek-v4-flash",
        name: "DeepSeek V4 Flash",
    },
    ModelSeed {
        id: "deepseek/deepseek-v4-pro",
        name: "DeepSeek V4 Pro",
    },
    ModelSeed {
        id: "deepseek/deepseek-chat-v3.1",
        name: "DeepSeek-V3.1",
    },
    ModelSeed {
        id: "deepseek/deepseek-r1",
        name: "DeepSeek: R1",
    },
    ModelSeed {
        id: "mistralai/devstral-2512",
        name: "Devstral 2 2512",
    },
    ModelSeed {
        id: "mistralai/devstral-medium-2507",
        name: "Devstral Medium",
    },
    ModelSeed {
        id: "mistralai/devstral-small-2505",
        name: "Devstral Small",
    },
    ModelSeed {
        id: "mistralai/devstral-small-2507",
        name: "Devstral Small 1.1",
    },
    ModelSeed {
        id: "openrouter/elephant-alpha",
        name: "Elephant (free)",
    },
    ModelSeed {
        id: "openrouter/free",
        name: "Free Models Router",
    },
    ModelSeed {
        id: "google/gemini-2.0-flash-001",
        name: "Gemini 2.0 Flash",
    },
    ModelSeed {
        id: "google/gemini-2.5-flash",
        name: "Gemini 2.5 Flash",
    },
    ModelSeed {
        id: "google/gemini-2.5-flash-lite",
        name: "Gemini 2.5 Flash Lite",
    },
    ModelSeed {
        id: "google/gemini-2.5-flash-lite-preview-09-2025",
        name: "Gemini 2.5 Flash Lite Preview 09-25",
    },
    ModelSeed {
        id: "google/gemini-2.5-flash-preview-09-2025",
        name: "Gemini 2.5 Flash Preview 09-25",
    },
    ModelSeed {
        id: "google/gemini-2.5-pro",
        name: "Gemini 2.5 Pro",
    },
    ModelSeed {
        id: "google/gemini-2.5-pro-preview-05-06",
        name: "Gemini 2.5 Pro Preview 05-06",
    },
    ModelSeed {
        id: "google/gemini-2.5-pro-preview-06-05",
        name: "Gemini 2.5 Pro Preview 06-05",
    },
    ModelSeed {
        id: "google/gemini-3-flash-preview",
        name: "Gemini 3 Flash Preview",
    },
    ModelSeed {
        id: "google/gemini-3-pro-preview",
        name: "Gemini 3 Pro Preview",
    },
    ModelSeed {
        id: "google/gemini-3.1-flash-lite-preview",
        name: "Gemini 3.1 Flash Lite Preview",
    },
    ModelSeed {
        id: "google/gemini-3.1-pro-preview",
        name: "Gemini 3.1 Pro Preview",
    },
    ModelSeed {
        id: "google/gemini-3.1-pro-preview-customtools",
        name: "Gemini 3.1 Pro Preview Custom Tools",
    },
    ModelSeed {
        id: "google/gemma-3-27b-it",
        name: "Gemma 3 27B",
    },
    ModelSeed {
        id: "google/gemma-3-27b-it:free",
        name: "Gemma 3 27B (free)",
    },
    ModelSeed {
        id: "google/gemma-4-26b-a4b-it",
        name: "Gemma 4 26B A4B",
    },
    ModelSeed {
        id: "google/gemma-4-26b-a4b-it:free",
        name: "Gemma 4 26B A4B (free)",
    },
    ModelSeed {
        id: "google/gemma-4-31b-it",
        name: "Gemma 4 31B",
    },
    ModelSeed {
        id: "google/gemma-4-31b-it:free",
        name: "Gemma 4 31B (free)",
    },
    ModelSeed {
        id: "z-ai/glm-4.5",
        name: "GLM 4.5",
    },
    ModelSeed {
        id: "z-ai/glm-4.5-air",
        name: "GLM 4.5 Air",
    },
    ModelSeed {
        id: "z-ai/glm-4.5v",
        name: "GLM 4.5V",
    },
    ModelSeed {
        id: "z-ai/glm-4.6",
        name: "GLM 4.6",
    },
    ModelSeed {
        id: "z-ai/glm-4.6:exacto",
        name: "GLM 4.6 (exacto)",
    },
    ModelSeed {
        id: "z-ai/glm-4.7",
        name: "GLM-4.7",
    },
    ModelSeed {
        id: "z-ai/glm-4.7-flash",
        name: "GLM-4.7-Flash",
    },
    ModelSeed {
        id: "z-ai/glm-5",
        name: "GLM-5",
    },
    ModelSeed {
        id: "z-ai/glm-5-turbo",
        name: "GLM-5-Turbo",
    },
    ModelSeed {
        id: "z-ai/glm-5.1",
        name: "GLM-5.1",
    },
    ModelSeed {
        id: "openai/gpt-oss-120b",
        name: "GPT OSS 120B",
    },
    ModelSeed {
        id: "openai/gpt-oss-120b:exacto",
        name: "GPT OSS 120B (exacto)",
    },
    ModelSeed {
        id: "openai/gpt-oss-20b",
        name: "GPT OSS 20B",
    },
    ModelSeed {
        id: "openai/gpt-oss-safeguard-20b",
        name: "GPT OSS Safeguard 20B",
    },
    ModelSeed {
        id: "openai/gpt-4.1",
        name: "GPT-4.1",
    },
    ModelSeed {
        id: "openai/gpt-4.1-mini",
        name: "GPT-4.1 Mini",
    },
    ModelSeed {
        id: "openai/gpt-4o-mini",
        name: "GPT-4o-mini",
    },
    ModelSeed {
        id: "openai/gpt-5",
        name: "GPT-5",
    },
    ModelSeed {
        id: "openai/gpt-5-codex",
        name: "GPT-5 Codex",
    },
    ModelSeed {
        id: "openai/gpt-5-image",
        name: "GPT-5 Image",
    },
    ModelSeed {
        id: "openai/gpt-5-mini",
        name: "GPT-5 Mini",
    },
    ModelSeed {
        id: "openai/gpt-5-nano",
        name: "GPT-5 Nano",
    },
    ModelSeed {
        id: "openai/gpt-5-pro",
        name: "GPT-5 Pro",
    },
    ModelSeed {
        id: "openai/gpt-5.1",
        name: "GPT-5.1",
    },
    ModelSeed {
        id: "openai/gpt-5.1-chat",
        name: "GPT-5.1 Chat",
    },
    ModelSeed {
        id: "openai/gpt-5.1-codex",
        name: "GPT-5.1-Codex",
    },
    ModelSeed {
        id: "openai/gpt-5.1-codex-max",
        name: "GPT-5.1-Codex-Max",
    },
    ModelSeed {
        id: "openai/gpt-5.1-codex-mini",
        name: "GPT-5.1-Codex-Mini",
    },
    ModelSeed {
        id: "openai/gpt-5.2",
        name: "GPT-5.2",
    },
    ModelSeed {
        id: "openai/gpt-5.2-chat",
        name: "GPT-5.2 Chat",
    },
    ModelSeed {
        id: "openai/gpt-5.2-pro",
        name: "GPT-5.2 Pro",
    },
    ModelSeed {
        id: "openai/gpt-5.2-codex",
        name: "GPT-5.2-Codex",
    },
    ModelSeed {
        id: "openai/gpt-5.3-codex",
        name: "GPT-5.3-Codex",
    },
    ModelSeed {
        id: "openai/gpt-5.4",
        name: "GPT-5.4",
    },
    ModelSeed {
        id: "openai/gpt-5.4-mini",
        name: "GPT-5.4 Mini",
    },
    ModelSeed {
        id: "openai/gpt-5.4-nano",
        name: "GPT-5.4 Nano",
    },
    ModelSeed {
        id: "openai/gpt-5.4-pro",
        name: "GPT-5.4 Pro",
    },
    ModelSeed {
        id: "openai/gpt-5.5",
        name: "GPT-5.5",
    },
    ModelSeed {
        id: "openai/gpt-5.5-pro",
        name: "GPT-5.5 Pro",
    },
    ModelSeed {
        id: "openai/gpt-oss-120b:free",
        name: "gpt-oss-120b (free)",
    },
    ModelSeed {
        id: "openai/gpt-oss-20b:free",
        name: "gpt-oss-20b (free)",
    },
    ModelSeed {
        id: "x-ai/grok-3",
        name: "Grok 3",
    },
    ModelSeed {
        id: "x-ai/grok-3-beta",
        name: "Grok 3 Beta",
    },
    ModelSeed {
        id: "x-ai/grok-3-mini",
        name: "Grok 3 Mini",
    },
    ModelSeed {
        id: "x-ai/grok-3-mini-beta",
        name: "Grok 3 Mini Beta",
    },
    ModelSeed {
        id: "x-ai/grok-4",
        name: "Grok 4",
    },
    ModelSeed {
        id: "x-ai/grok-4-fast",
        name: "Grok 4 Fast",
    },
    ModelSeed {
        id: "x-ai/grok-4.1-fast",
        name: "Grok 4.1 Fast",
    },
    ModelSeed {
        id: "x-ai/grok-4.20-beta",
        name: "Grok 4.20 Beta",
    },
    ModelSeed {
        id: "x-ai/grok-code-fast-1",
        name: "Grok Code Fast 1",
    },
    ModelSeed {
        id: "nousresearch/hermes-4-405b",
        name: "Hermes 4 405B",
    },
    ModelSeed {
        id: "nousresearch/hermes-4-70b",
        name: "Hermes 4 70B",
    },
    ModelSeed {
        id: "prime-intellect/intellect-3",
        name: "Intellect 3",
    },
    ModelSeed {
        id: "moonshotai/kimi-k2",
        name: "Kimi K2",
    },
    ModelSeed {
        id: "moonshotai/kimi-k2-0905",
        name: "Kimi K2 Instruct 0905",
    },
    ModelSeed {
        id: "moonshotai/kimi-k2-0905:exacto",
        name: "Kimi K2 Instruct 0905 (exacto)",
    },
    ModelSeed {
        id: "moonshotai/kimi-k2-thinking",
        name: "Kimi K2 Thinking",
    },
    ModelSeed {
        id: "moonshotai/kimi-k2.5",
        name: "Kimi K2.5",
    },
    ModelSeed {
        id: "moonshotai/kimi-k2.6",
        name: "Kimi K2.6",
    },
    ModelSeed {
        id: "meta-llama/llama-3.3-70b-instruct:free",
        name: "Llama 3.3 70B Instruct (free)",
    },
    ModelSeed {
        id: "inception/mercury-2",
        name: "Mercury 2",
    },
    ModelSeed {
        id: "xiaomi/mimo-v2-flash",
        name: "MiMo-V2-Flash",
    },
    ModelSeed {
        id: "xiaomi/mimo-v2-omni",
        name: "MiMo-V2-Omni",
    },
    ModelSeed {
        id: "xiaomi/mimo-v2-pro",
        name: "MiMo-V2-Pro",
    },
    ModelSeed {
        id: "xiaomi/mimo-v2.5",
        name: "MiMo-V2.5",
    },
    ModelSeed {
        id: "xiaomi/mimo-v2.5-pro",
        name: "MiMo-V2.5-Pro",
    },
    ModelSeed {
        id: "minimax/minimax-m1",
        name: "MiniMax M1",
    },
    ModelSeed {
        id: "minimax/minimax-m2",
        name: "MiniMax M2",
    },
    ModelSeed {
        id: "minimax/minimax-m2.1",
        name: "MiniMax M2.1",
    },
    ModelSeed {
        id: "minimax/minimax-m2.5",
        name: "MiniMax M2.5",
    },
    ModelSeed {
        id: "minimax/minimax-m2.5:free",
        name: "MiniMax M2.5 (free)",
    },
    ModelSeed {
        id: "minimax/minimax-m2.7",
        name: "MiniMax M2.7",
    },
    ModelSeed {
        id: "minimax/minimax-01",
        name: "MiniMax-01",
    },
    ModelSeed {
        id: "mistralai/mistral-medium-3",
        name: "Mistral Medium 3",
    },
    ModelSeed {
        id: "mistralai/mistral-medium-3.1",
        name: "Mistral Medium 3.1",
    },
    ModelSeed {
        id: "mistralai/mistral-small-3.1-24b-instruct",
        name: "Mistral Small 3.1 24B Instruct",
    },
    ModelSeed {
        id: "mistralai/mistral-small-3.2-24b-instruct",
        name: "Mistral Small 3.2 24B Instruct",
    },
    ModelSeed {
        id: "mistralai/mistral-small-2603",
        name: "Mistral Small 4",
    },
    ModelSeed {
        id: "nvidia/nemotron-3-nano-30b-a3b:free",
        name: "Nemotron 3 Nano 30B A3B (free)",
    },
    ModelSeed {
        id: "nvidia/nemotron-3-nano-omni-30b-a3b-reasoning:free",
        name: "Nemotron 3 Nano Omni (free)",
    },
    ModelSeed {
        id: "nvidia/nemotron-3-super-120b-a12b",
        name: "Nemotron 3 Super",
    },
    ModelSeed {
        id: "nvidia/nemotron-3-super-120b-a12b:free",
        name: "Nemotron 3 Super (free)",
    },
    ModelSeed {
        id: "nvidia/nemotron-nano-12b-v2-vl:free",
        name: "Nemotron Nano 12B 2 VL (free)",
    },
    ModelSeed {
        id: "nvidia/nemotron-nano-9b-v2:free",
        name: "Nemotron Nano 9B V2 (free)",
    },
    ModelSeed {
        id: "nvidia/nemotron-nano-9b-v2",
        name: "nvidia-nemotron-nano-9b-v2",
    },
    ModelSeed {
        id: "openai/o4-mini",
        name: "o4 Mini",
    },
    ModelSeed {
        id: "openrouter/owl-alpha",
        name: "Owl Alpha",
    },
    ModelSeed {
        id: "openrouter/pareto-code",
        name: "Pareto Code Router",
    },
    ModelSeed {
        id: "qwen/qwen3.5-flash-02-23",
        name: "Qwen: Qwen3.5-Flash",
    },
    ModelSeed {
        id: "qwen/qwen3-235b-a22b-07-25",
        name: "Qwen3 235B A22B Instruct 2507",
    },
    ModelSeed {
        id: "qwen/qwen3-235b-a22b-thinking-2507",
        name: "Qwen3 235B A22B Thinking 2507",
    },
    ModelSeed {
        id: "qwen/qwen3-30b-a3b-instruct-2507",
        name: "Qwen3 30B A3B Instruct 2507",
    },
    ModelSeed {
        id: "qwen/qwen3-30b-a3b-thinking-2507",
        name: "Qwen3 30B A3B Thinking 2507",
    },
    ModelSeed {
        id: "qwen/qwen3-coder",
        name: "Qwen3 Coder",
    },
    ModelSeed {
        id: "qwen/qwen3-coder:exacto",
        name: "Qwen3 Coder (exacto)",
    },
    ModelSeed {
        id: "qwen/qwen3-coder-30b-a3b-instruct",
        name: "Qwen3 Coder 30B A3B Instruct",
    },
    ModelSeed {
        id: "qwen/qwen3-coder-flash",
        name: "Qwen3 Coder Flash",
    },
    ModelSeed {
        id: "qwen/qwen3-max",
        name: "Qwen3 Max",
    },
    ModelSeed {
        id: "qwen/qwen3-next-80b-a3b-instruct",
        name: "Qwen3 Next 80B A3B Instruct",
    },
    ModelSeed {
        id: "qwen/qwen3-next-80b-a3b-thinking",
        name: "Qwen3 Next 80B A3B Thinking",
    },
    ModelSeed {
        id: "qwen/qwen3.5-397b-a17b",
        name: "Qwen3.5 397B A17B",
    },
    ModelSeed {
        id: "qwen/qwen3.5-plus-02-15",
        name: "Qwen3.5 Plus 2026-02-15",
    },
    ModelSeed {
        id: "qwen/qwen3.6-plus",
        name: "Qwen3.6 Plus",
    },
    ModelSeed {
        id: "stepfun/step-3.5-flash",
        name: "Step 3.5 Flash",
    },
    ModelSeed {
        id: "arcee-ai/trinity-large-preview:free",
        name: "Trinity Large Preview",
    },
    ModelSeed {
        id: "arcee-ai/trinity-large-thinking",
        name: "Trinity Large Thinking",
    },
];
pub static EMPTY_MODELS: &[ModelSeed] = &[];
pub static OLLAMA_SEED_MODELS: &[ModelSeed] = &[
    ModelSeed {
        id: "qwen3.5:4b",
        name: "Qwen3.5 4B",
    },
    ModelSeed {
        id: "qwen3.5:0.8b",
        name: "Qwen3.5 0.8B",
    },
    ModelSeed {
        id: "qwen3.5:9b",
        name: "Qwen3.5 9B",
    },
];
pub static LM_STUDIO_SEED_MODELS: &[ModelSeed] = &[
    ModelSeed {
        id: "qwen/qwen3.5-4b",
        name: "Qwen3.5 4B",
    },
    ModelSeed {
        id: "qwen/qwen3.5-0.8b",
        name: "Qwen3.5 0.8B",
    },
    ModelSeed {
        id: "qwen/qwen3.5-9b",
        name: "Qwen3.5 9B",
    },
];
pub static NVIDIA_SEED_MODELS: &[ModelSeed] = &[
    ModelSeed {
        id: "01-ai/yi-large",
        name: "01-ai/yi-large",
    },
    ModelSeed {
        id: "abacusai/dracarys-llama-3.1-70b-instruct",
        name: "abacusai/dracarys-llama-3.1-70b-instruct",
    },
    ModelSeed {
        id: "adept/fuyu-8b",
        name: "adept/fuyu-8b",
    },
    ModelSeed {
        id: "ai21labs/jamba-1.5-large-instruct",
        name: "ai21labs/jamba-1.5-large-instruct",
    },
    ModelSeed {
        id: "aisingapore/sea-lion-7b-instruct",
        name: "aisingapore/sea-lion-7b-instruct",
    },
    ModelSeed {
        id: "baai/bge-m3",
        name: "baai/bge-m3",
    },
    ModelSeed {
        id: "bigcode/starcoder2-15b",
        name: "bigcode/starcoder2-15b",
    },
    ModelSeed {
        id: "bytedance/seed-oss-36b-instruct",
        name: "bytedance/seed-oss-36b-instruct",
    },
    ModelSeed {
        id: "databricks/dbrx-instruct",
        name: "databricks/dbrx-instruct",
    },
    ModelSeed {
        id: "deepseek-ai/deepseek-coder-6.7b-instruct",
        name: "deepseek-ai/deepseek-coder-6.7b-instruct",
    },
    ModelSeed {
        id: "deepseek-ai/deepseek-v4-flash",
        name: "deepseek-ai/deepseek-v4-flash",
    },
    ModelSeed {
        id: "deepseek-ai/deepseek-v4-pro",
        name: "deepseek-ai/deepseek-v4-pro",
    },
    ModelSeed {
        id: "google/codegemma-1.1-7b",
        name: "google/codegemma-1.1-7b",
    },
    ModelSeed {
        id: "google/codegemma-7b",
        name: "google/codegemma-7b",
    },
    ModelSeed {
        id: "google/deplot",
        name: "google/deplot",
    },
    ModelSeed {
        id: "google/gemma-2-2b-it",
        name: "google/gemma-2-2b-it",
    },
    ModelSeed {
        id: "google/gemma-2b",
        name: "google/gemma-2b",
    },
    ModelSeed {
        id: "google/gemma-3-12b-it",
        name: "google/gemma-3-12b-it",
    },
    ModelSeed {
        id: "google/gemma-3-4b-it",
        name: "google/gemma-3-4b-it",
    },
    ModelSeed {
        id: "google/gemma-3n-e2b-it",
        name: "google/gemma-3n-e2b-it",
    },
    ModelSeed {
        id: "google/gemma-3n-e4b-it",
        name: "google/gemma-3n-e4b-it",
    },
    ModelSeed {
        id: "google/gemma-4-31b-it",
        name: "google/gemma-4-31b-it",
    },
    ModelSeed {
        id: "google/recurrentgemma-2b",
        name: "google/recurrentgemma-2b",
    },
    ModelSeed {
        id: "ibm/granite-3.0-3b-a800m-instruct",
        name: "ibm/granite-3.0-3b-a800m-instruct",
    },
    ModelSeed {
        id: "ibm/granite-3.0-8b-instruct",
        name: "ibm/granite-3.0-8b-instruct",
    },
    ModelSeed {
        id: "ibm/granite-34b-code-instruct",
        name: "ibm/granite-34b-code-instruct",
    },
    ModelSeed {
        id: "ibm/granite-8b-code-instruct",
        name: "ibm/granite-8b-code-instruct",
    },
    ModelSeed {
        id: "meta/codellama-70b",
        name: "meta/codellama-70b",
    },
    ModelSeed {
        id: "meta/llama-3.1-70b-instruct",
        name: "meta/llama-3.1-70b-instruct",
    },
    ModelSeed {
        id: "meta/llama-3.1-8b-instruct",
        name: "meta/llama-3.1-8b-instruct",
    },
    ModelSeed {
        id: "meta/llama-3.2-11b-vision-instruct",
        name: "meta/llama-3.2-11b-vision-instruct",
    },
    ModelSeed {
        id: "meta/llama-3.2-1b-instruct",
        name: "meta/llama-3.2-1b-instruct",
    },
    ModelSeed {
        id: "meta/llama-3.2-3b-instruct",
        name: "meta/llama-3.2-3b-instruct",
    },
    ModelSeed {
        id: "meta/llama-3.2-90b-vision-instruct",
        name: "meta/llama-3.2-90b-vision-instruct",
    },
    ModelSeed {
        id: "meta/llama-3.3-70b-instruct",
        name: "meta/llama-3.3-70b-instruct",
    },
    ModelSeed {
        id: "meta/llama-4-maverick-17b-128e-instruct",
        name: "meta/llama-4-maverick-17b-128e-instruct",
    },
    ModelSeed {
        id: "meta/llama-guard-4-12b",
        name: "meta/llama-guard-4-12b",
    },
    ModelSeed {
        id: "meta/llama2-70b",
        name: "meta/llama2-70b",
    },
    ModelSeed {
        id: "microsoft/kosmos-2",
        name: "microsoft/kosmos-2",
    },
    ModelSeed {
        id: "microsoft/phi-3-vision-128k-instruct",
        name: "microsoft/phi-3-vision-128k-instruct",
    },
    ModelSeed {
        id: "microsoft/phi-3.5-moe-instruct",
        name: "microsoft/phi-3.5-moe-instruct",
    },
    ModelSeed {
        id: "microsoft/phi-4-mini-instruct",
        name: "microsoft/phi-4-mini-instruct",
    },
    ModelSeed {
        id: "microsoft/phi-4-multimodal-instruct",
        name: "microsoft/phi-4-multimodal-instruct",
    },
    ModelSeed {
        id: "minimaxai/minimax-m2.7",
        name: "minimaxai/minimax-m2.7",
    },
    ModelSeed {
        id: "mistralai/codestral-22b-instruct-v0.1",
        name: "mistralai/codestral-22b-instruct-v0.1",
    },
    ModelSeed {
        id: "mistralai/ministral-14b-instruct-2512",
        name: "mistralai/ministral-14b-instruct-2512",
    },
    ModelSeed {
        id: "mistralai/mistral-7b-instruct-v0.3",
        name: "mistralai/mistral-7b-instruct-v0.3",
    },
    ModelSeed {
        id: "mistralai/mistral-large",
        name: "mistralai/mistral-large",
    },
    ModelSeed {
        id: "mistralai/mistral-large-2-instruct",
        name: "mistralai/mistral-large-2-instruct",
    },
    ModelSeed {
        id: "mistralai/mistral-large-3-675b-instruct-2512",
        name: "mistralai/mistral-large-3-675b-instruct-2512",
    },
    ModelSeed {
        id: "mistralai/mistral-medium-3.5-128b",
        name: "mistralai/mistral-medium-3.5-128b",
    },
    ModelSeed {
        id: "mistralai/mistral-nemotron",
        name: "mistralai/mistral-nemotron",
    },
    ModelSeed {
        id: "mistralai/mistral-small-4-119b-2603",
        name: "mistralai/mistral-small-4-119b-2603",
    },
    ModelSeed {
        id: "mistralai/mixtral-8x22b-v0.1",
        name: "mistralai/mixtral-8x22b-v0.1",
    },
    ModelSeed {
        id: "mistralai/mixtral-8x7b-instruct-v0.1",
        name: "mistralai/mixtral-8x7b-instruct-v0.1",
    },
    ModelSeed {
        id: "moonshotai/kimi-k2.6",
        name: "moonshotai/kimi-k2.6",
    },
    ModelSeed {
        id: "nv-mistralai/mistral-nemo-12b-instruct",
        name: "nv-mistralai/mistral-nemo-12b-instruct",
    },
    ModelSeed {
        id: "nvidia/ai-synthetic-video-detector",
        name: "nvidia/ai-synthetic-video-detector",
    },
    ModelSeed {
        id: "nvidia/cosmos-reason2-8b",
        name: "nvidia/cosmos-reason2-8b",
    },
    ModelSeed {
        id: "nvidia/embed-qa-4",
        name: "nvidia/embed-qa-4",
    },
    ModelSeed {
        id: "nvidia/gliner-pii",
        name: "nvidia/gliner-pii",
    },
    ModelSeed {
        id: "nvidia/ising-calibration-1-35b-a3b",
        name: "nvidia/ising-calibration-1-35b-a3b",
    },
    ModelSeed {
        id: "nvidia/llama-3.1-nemoguard-8b-content-safety",
        name: "nvidia/llama-3.1-nemoguard-8b-content-safety",
    },
    ModelSeed {
        id: "nvidia/llama-3.1-nemoguard-8b-topic-control",
        name: "nvidia/llama-3.1-nemoguard-8b-topic-control",
    },
    ModelSeed {
        id: "nvidia/llama-3.1-nemotron-51b-instruct",
        name: "nvidia/llama-3.1-nemotron-51b-instruct",
    },
    ModelSeed {
        id: "nvidia/llama-3.1-nemotron-70b-instruct",
        name: "nvidia/llama-3.1-nemotron-70b-instruct",
    },
    ModelSeed {
        id: "nvidia/llama-3.1-nemotron-nano-8b-v1",
        name: "nvidia/llama-3.1-nemotron-nano-8b-v1",
    },
    ModelSeed {
        id: "nvidia/llama-3.1-nemotron-nano-vl-8b-v1",
        name: "nvidia/llama-3.1-nemotron-nano-vl-8b-v1",
    },
    ModelSeed {
        id: "nvidia/llama-3.1-nemotron-safety-guard-8b-v3",
        name: "nvidia/llama-3.1-nemotron-safety-guard-8b-v3",
    },
    ModelSeed {
        id: "nvidia/llama-3.1-nemotron-ultra-253b-v1",
        name: "nvidia/llama-3.1-nemotron-ultra-253b-v1",
    },
    ModelSeed {
        id: "nvidia/llama-3.2-nemoretriever-1b-vlm-embed-v1",
        name: "nvidia/llama-3.2-nemoretriever-1b-vlm-embed-v1",
    },
    ModelSeed {
        id: "nvidia/llama-3.2-nv-embedqa-1b-v1",
        name: "nvidia/llama-3.2-nv-embedqa-1b-v1",
    },
    ModelSeed {
        id: "nvidia/llama-3.3-nemotron-super-49b-v1",
        name: "nvidia/llama-3.3-nemotron-super-49b-v1",
    },
    ModelSeed {
        id: "nvidia/llama-3.3-nemotron-super-49b-v1.5",
        name: "nvidia/llama-3.3-nemotron-super-49b-v1.5",
    },
    ModelSeed {
        id: "nvidia/llama-nemotron-embed-1b-v2",
        name: "nvidia/llama-nemotron-embed-1b-v2",
    },
    ModelSeed {
        id: "nvidia/llama-nemotron-embed-vl-1b-v2",
        name: "nvidia/llama-nemotron-embed-vl-1b-v2",
    },
    ModelSeed {
        id: "nvidia/llama3-chatqa-1.5-70b",
        name: "nvidia/llama3-chatqa-1.5-70b",
    },
    ModelSeed {
        id: "nvidia/mistral-nemo-minitron-8b-8k-instruct",
        name: "nvidia/mistral-nemo-minitron-8b-8k-instruct",
    },
    ModelSeed {
        id: "nvidia/nemoretriever-parse",
        name: "nvidia/nemoretriever-parse",
    },
    ModelSeed {
        id: "nvidia/nemotron-3-content-safety",
        name: "nvidia/nemotron-3-content-safety",
    },
    ModelSeed {
        id: "nvidia/nemotron-3-nano-30b-a3b",
        name: "nvidia/nemotron-3-nano-30b-a3b",
    },
    ModelSeed {
        id: "nvidia/nemotron-3-nano-omni-30b-a3b-reasoning",
        name: "nvidia/nemotron-3-nano-omni-30b-a3b-reasoning",
    },
    ModelSeed {
        id: "nvidia/nemotron-3-super-120b-a12b",
        name: "nvidia/nemotron-3-super-120b-a12b",
    },
    ModelSeed {
        id: "nvidia/nemotron-4-340b-instruct",
        name: "nvidia/nemotron-4-340b-instruct",
    },
    ModelSeed {
        id: "nvidia/nemotron-4-340b-reward",
        name: "nvidia/nemotron-4-340b-reward",
    },
    ModelSeed {
        id: "nvidia/nemotron-content-safety-reasoning-4b",
        name: "nvidia/nemotron-content-safety-reasoning-4b",
    },
    ModelSeed {
        id: "nvidia/nemotron-mini-4b-instruct",
        name: "nvidia/nemotron-mini-4b-instruct",
    },
    ModelSeed {
        id: "nvidia/nemotron-nano-12b-v2-vl",
        name: "nvidia/nemotron-nano-12b-v2-vl",
    },
    ModelSeed {
        id: "nvidia/nemotron-nano-3-30b-a3b",
        name: "nvidia/nemotron-nano-3-30b-a3b",
    },
    ModelSeed {
        id: "nvidia/nemotron-parse",
        name: "nvidia/nemotron-parse",
    },
    ModelSeed {
        id: "nvidia/neva-22b",
        name: "nvidia/neva-22b",
    },
    ModelSeed {
        id: "nvidia/nv-embed-v1",
        name: "nvidia/nv-embed-v1",
    },
    ModelSeed {
        id: "nvidia/nv-embedcode-7b-v1",
        name: "nvidia/nv-embedcode-7b-v1",
    },
    ModelSeed {
        id: "nvidia/nv-embedqa-e5-v5",
        name: "nvidia/nv-embedqa-e5-v5",
    },
    ModelSeed {
        id: "nvidia/nv-embedqa-mistral-7b-v2",
        name: "nvidia/nv-embedqa-mistral-7b-v2",
    },
    ModelSeed {
        id: "nvidia/nvclip",
        name: "nvidia/nvclip",
    },
    ModelSeed {
        id: "nvidia/nvidia-nemotron-nano-9b-v2",
        name: "nvidia/nvidia-nemotron-nano-9b-v2",
    },
    ModelSeed {
        id: "nvidia/riva-translate-4b-instruct",
        name: "nvidia/riva-translate-4b-instruct",
    },
    ModelSeed {
        id: "nvidia/riva-translate-4b-instruct-v1.1",
        name: "nvidia/riva-translate-4b-instruct-v1.1",
    },
    ModelSeed {
        id: "nvidia/vila",
        name: "nvidia/vila",
    },
    ModelSeed {
        id: "openai/gpt-oss-120b",
        name: "openai/gpt-oss-120b",
    },
    ModelSeed {
        id: "openai/gpt-oss-20b",
        name: "openai/gpt-oss-20b",
    },
    ModelSeed {
        id: "qwen/qwen3-coder-480b-a35b-instruct",
        name: "qwen/qwen3-coder-480b-a35b-instruct",
    },
    ModelSeed {
        id: "qwen/qwen3-next-80b-a3b-instruct",
        name: "qwen/qwen3-next-80b-a3b-instruct",
    },
    ModelSeed {
        id: "qwen/qwen3.5-122b-a10b",
        name: "qwen/qwen3.5-122b-a10b",
    },
    ModelSeed {
        id: "qwen/qwen3.5-397b-a17b",
        name: "qwen/qwen3.5-397b-a17b",
    },
    ModelSeed {
        id: "sarvamai/sarvam-m",
        name: "sarvamai/sarvam-m",
    },
    ModelSeed {
        id: "snowflake/arctic-embed-l",
        name: "snowflake/arctic-embed-l",
    },
    ModelSeed {
        id: "stepfun-ai/step-3.5-flash",
        name: "stepfun-ai/step-3.5-flash",
    },
    ModelSeed {
        id: "stockmark/stockmark-2-100b-instruct",
        name: "stockmark/stockmark-2-100b-instruct",
    },
    ModelSeed {
        id: "upstage/solar-10.7b-instruct",
        name: "upstage/solar-10.7b-instruct",
    },
    ModelSeed {
        id: "writer/palmyra-creative-122b",
        name: "writer/palmyra-creative-122b",
    },
    ModelSeed {
        id: "writer/palmyra-fin-70b-32k",
        name: "writer/palmyra-fin-70b-32k",
    },
    ModelSeed {
        id: "writer/palmyra-med-70b",
        name: "writer/palmyra-med-70b",
    },
    ModelSeed {
        id: "writer/palmyra-med-70b-32k",
        name: "writer/palmyra-med-70b-32k",
    },
    ModelSeed {
        id: "z-ai/glm-5.1",
        name: "z-ai/glm-5.1",
    },
    ModelSeed {
        id: "zyphra/zamba2-7b-instruct",
        name: "zyphra/zamba2-7b-instruct",
    },
];
pub static OLLAMA_CLOUD_SEED_MODELS: &[ModelSeed] = &[];
pub static FIREWORKS_SEED_MODELS: &[ModelSeed] = &[];
pub static XAI_SEED_MODELS: &[ModelSeed] = &[];
pub static XIAOMI_SEED_MODELS: &[ModelSeed] = &[];
pub static GOOGLE_SEED_MODELS: &[ModelSeed] = &[
    ModelSeed {
        id: "gemini-2.5-pro",
        name: "Gemini 2.5 Pro",
    },
    ModelSeed {
        id: "gemini-2.5-flash",
        name: "Gemini 2.5 Flash",
    },
];
pub static CUSTOM_SEED_MODELS: &[ModelSeed] = &[];

#[derive(Debug, Clone, Copy)]
pub struct ProviderCatalogEntry {
    pub id: &'static str,
    pub aliases: &'static [&'static str],
    pub label: &'static str,
    pub kind: ProviderKind,
    pub default_base_url: Option<&'static str>,
    pub base_url_env: Option<&'static str>,
    pub api_key_env: Option<&'static str>,
    pub credential_required: bool,
    pub endpoint_override_supported: bool,
    pub default_wire_mode: WireMode,
    pub supported_wire_modes: &'static [WireMode],
    pub default_model: Option<&'static str>,
    pub models: &'static [ModelSeed],
    pub allows_custom_model_id: bool,
    pub model_discovery_supported: bool,
    pub extra_request_passthrough: bool,
}

pub static OPENAI_CHAT: &[WireMode] = &[WireMode::OpenAiChatCompletions];
pub static OPENAI_CHAT_RESPONSES: &[WireMode] =
    &[WireMode::OpenAiChatCompletions, WireMode::OpenAiResponses];
pub static ANTHROPIC_WIRES: &[WireMode] = &[WireMode::AnthropicMessages];
pub static GOOGLE_WIRES: &[WireMode] = &[WireMode::GoogleGemini];
pub static CODEX_WIRES: &[WireMode] = &[WireMode::CodexOAuthRuntime];
pub static OLLAMA_WIRES: &[WireMode] = &[WireMode::OllamaNative, WireMode::OpenAiChatCompletions];
pub static XIAOMI_WIRES: &[WireMode] =
    &[WireMode::OpenAiChatCompletions, WireMode::AnthropicMessages];

pub static PROVIDERS: &[ProviderCatalogEntry] = &[
    ProviderCatalogEntry {
        id: "ollama",
        aliases: &[],
        label: "Ollama",
        kind: ProviderKind::Local,
        default_base_url: Some("http://127.0.0.1:11434"),
        base_url_env: Some("OLLAMA_BASE_URL"),
        api_key_env: None,
        credential_required: false,
        endpoint_override_supported: true,
        default_wire_mode: WireMode::OllamaNative,
        supported_wire_modes: OLLAMA_WIRES,
        default_model: Some("qwen3.5:0.8b"),
        models: OLLAMA_SEED_MODELS,
        allows_custom_model_id: true,
        model_discovery_supported: true,
        extra_request_passthrough: false,
    },
    ProviderCatalogEntry {
        id: "lm-studio",
        aliases: &["lmstudio"],
        label: "LM Studio",
        kind: ProviderKind::Local,
        default_base_url: Some("http://127.0.0.1:1234/v1"),
        base_url_env: Some("LMSTUDIO_BASE_URL"),
        api_key_env: None,
        credential_required: false,
        endpoint_override_supported: true,
        default_wire_mode: WireMode::OpenAiChatCompletions,
        supported_wire_modes: OPENAI_CHAT_RESPONSES,
        default_model: None,
        models: LM_STUDIO_SEED_MODELS,
        allows_custom_model_id: true,
        model_discovery_supported: true,
        extra_request_passthrough: false,
    },
    ProviderCatalogEntry {
        id: "openai",
        aliases: &["openai-api", "openai-compat"],
        label: "OpenAI",
        kind: ProviderKind::ApiKey,
        default_base_url: Some("https://api.openai.com/v1"),
        base_url_env: Some("OPENAI_BASE_URL"),
        api_key_env: Some("OPENAI_API_KEY"),
        credential_required: true,
        endpoint_override_supported: true,
        default_wire_mode: WireMode::OpenAiChatCompletions,
        supported_wire_modes: OPENAI_CHAT_RESPONSES,
        default_model: Some("gpt-5.1"),
        models: OPENAI_MODELS,
        allows_custom_model_id: true,
        model_discovery_supported: true,
        extra_request_passthrough: true,
    },
    ProviderCatalogEntry {
        id: "anthropic",
        aliases: &[],
        label: "Anthropic",
        kind: ProviderKind::ApiKey,
        default_base_url: Some("https://api.anthropic.com/v1"),
        base_url_env: Some("ANTHROPIC_BASE_URL"),
        api_key_env: Some("ANTHROPIC_API_KEY"),
        credential_required: true,
        endpoint_override_supported: true,
        default_wire_mode: WireMode::AnthropicMessages,
        supported_wire_modes: ANTHROPIC_WIRES,
        default_model: Some("claude-sonnet-4-5-20250929"),
        models: ANTHROPIC_MODELS,
        allows_custom_model_id: true,
        model_discovery_supported: true,
        extra_request_passthrough: true,
    },
    ProviderCatalogEntry {
        id: "google",
        aliases: &["gemini"],
        label: "Google Gemini",
        kind: ProviderKind::ApiKey,
        default_base_url: Some("https://generativelanguage.googleapis.com/v1beta"),
        base_url_env: Some("GOOGLE_BASE_URL"),
        api_key_env: Some("GOOGLE_API_KEY"),
        credential_required: true,
        endpoint_override_supported: true,
        default_wire_mode: WireMode::GoogleGemini,
        supported_wire_modes: GOOGLE_WIRES,
        default_model: Some("gemini-2.5-flash"),
        models: GOOGLE_SEED_MODELS,
        allows_custom_model_id: true,
        model_discovery_supported: true,
        extra_request_passthrough: true,
    },
    ProviderCatalogEntry {
        id: "codex-oauth",
        aliases: &["codex"],
        label: "Codex OAuth",
        kind: ProviderKind::OAuth,
        default_base_url: Some("http://127.0.0.1:9110"),
        base_url_env: Some("CODEX_OAUTH_RUNTIME_URL"),
        api_key_env: None,
        credential_required: false,
        endpoint_override_supported: true,
        default_wire_mode: WireMode::CodexOAuthRuntime,
        supported_wire_modes: CODEX_WIRES,
        default_model: None,
        models: EMPTY_MODELS,
        allows_custom_model_id: true,
        model_discovery_supported: true,
        extra_request_passthrough: false,
    },
    ProviderCatalogEntry {
        id: "openrouter",
        aliases: &[],
        label: "OpenRouter",
        kind: ProviderKind::ApiKey,
        default_base_url: Some("https://openrouter.ai/api/v1"),
        base_url_env: Some("OPENROUTER_BASE_URL"),
        api_key_env: Some("OPENROUTER_API_KEY"),
        credential_required: true,
        endpoint_override_supported: true,
        default_wire_mode: WireMode::OpenAiChatCompletions,
        supported_wire_modes: OPENAI_CHAT_RESPONSES,
        default_model: Some("anthropic/claude-sonnet-4.6"),
        models: OPENROUTER_MODELS,
        allows_custom_model_id: true,
        model_discovery_supported: true,
        extra_request_passthrough: true,
    },
    ProviderCatalogEntry {
        id: "groq",
        aliases: &[],
        label: "Groq",
        kind: ProviderKind::ApiKey,
        default_base_url: Some("https://api.groq.com/openai/v1"),
        base_url_env: Some("GROQ_BASE_URL"),
        api_key_env: Some("GROQ_API_KEY"),
        credential_required: true,
        endpoint_override_supported: true,
        default_wire_mode: WireMode::OpenAiChatCompletions,
        supported_wire_modes: OPENAI_CHAT_RESPONSES,
        default_model: Some("openai/gpt-oss-120b"),
        models: GROQ_MODELS,
        allows_custom_model_id: true,
        model_discovery_supported: true,
        extra_request_passthrough: true,
    },
    ProviderCatalogEntry {
        id: "xai",
        aliases: &["x-ai"],
        label: "xAI",
        kind: ProviderKind::ApiKey,
        default_base_url: Some("https://api.x.ai/v1"),
        base_url_env: Some("XAI_BASE_URL"),
        api_key_env: Some("XAI_API_KEY"),
        credential_required: true,
        endpoint_override_supported: true,
        default_wire_mode: WireMode::OpenAiChatCompletions,
        supported_wire_modes: OPENAI_CHAT,
        default_model: None,
        models: XAI_SEED_MODELS,
        allows_custom_model_id: true,
        model_discovery_supported: true,
        extra_request_passthrough: true,
    },
    ProviderCatalogEntry {
        id: "fireworks",
        aliases: &["fireworks-ai"],
        label: "Fireworks AI",
        kind: ProviderKind::ApiKey,
        default_base_url: Some("https://api.fireworks.ai/inference/v1"),
        base_url_env: Some("FIREWORKS_BASE_URL"),
        api_key_env: Some("FIREWORKS_API_KEY"),
        credential_required: true,
        endpoint_override_supported: true,
        default_wire_mode: WireMode::OpenAiChatCompletions,
        supported_wire_modes: OPENAI_CHAT_RESPONSES,
        default_model: None,
        models: FIREWORKS_SEED_MODELS,
        allows_custom_model_id: true,
        model_discovery_supported: true,
        extra_request_passthrough: true,
    },
    ProviderCatalogEntry {
        id: "nvidia",
        aliases: &[],
        label: "NVIDIA",
        kind: ProviderKind::ApiKey,
        default_base_url: Some("https://integrate.api.nvidia.com/v1"),
        base_url_env: Some("NVIDIA_BASE_URL"),
        api_key_env: Some("NVIDIA_API_KEY"),
        credential_required: true,
        endpoint_override_supported: true,
        default_wire_mode: WireMode::OpenAiChatCompletions,
        supported_wire_modes: OPENAI_CHAT,
        default_model: Some("moonshotai/kimi-k2.6"),
        models: NVIDIA_SEED_MODELS,
        allows_custom_model_id: true,
        model_discovery_supported: true,
        extra_request_passthrough: true,
    },
    ProviderCatalogEntry {
        id: "ollama-cloud",
        aliases: &[],
        label: "Ollama Cloud",
        kind: ProviderKind::ApiKey,
        default_base_url: Some("https://ollama.com/v1"),
        base_url_env: Some("OLLAMA_CLOUD_BASE_URL"),
        api_key_env: Some("OLLAMA_API_KEY"),
        credential_required: true,
        endpoint_override_supported: true,
        default_wire_mode: WireMode::OpenAiResponses,
        supported_wire_modes: OPENAI_CHAT_RESPONSES,
        default_model: None,
        models: OLLAMA_CLOUD_SEED_MODELS,
        allows_custom_model_id: true,
        model_discovery_supported: true,
        extra_request_passthrough: true,
    },
    ProviderCatalogEntry {
        id: "xiaomi",
        aliases: &["mimo", "xiaomimimo", "xiaomi-mimo", "token-plan"],
        label: "XIAOMI",
        kind: ProviderKind::ApiKey,
        default_base_url: Some("https://token-plan-sgp.xiaomimimo.com/v1"),
        base_url_env: Some("XIAOMI_BASE_URL"),
        api_key_env: Some("XIAOMI_API_KEY"),
        credential_required: true,
        endpoint_override_supported: true,
        default_wire_mode: WireMode::OpenAiChatCompletions,
        supported_wire_modes: XIAOMI_WIRES,
        default_model: None,
        models: XIAOMI_SEED_MODELS,
        allows_custom_model_id: true,
        model_discovery_supported: true,
        extra_request_passthrough: true,
    },
    ProviderCatalogEntry {
        id: "custom",
        aliases: &[],
        label: "Custom Endpoint",
        kind: ProviderKind::Custom,
        default_base_url: None,
        base_url_env: None,
        api_key_env: None,
        credential_required: false,
        endpoint_override_supported: true,
        default_wire_mode: WireMode::OpenAiChatCompletions,
        supported_wire_modes: OPENAI_CHAT_RESPONSES,
        default_model: None,
        models: CUSTOM_SEED_MODELS,
        allows_custom_model_id: true,
        model_discovery_supported: true,
        extra_request_passthrough: true,
    },
];

pub fn provider_catalog() -> &'static [ProviderCatalogEntry] {
    PROVIDERS
}

pub fn find_provider(id_or_alias: &str) -> Option<&'static ProviderCatalogEntry> {
    let normalized = id_or_alias.trim().to_ascii_lowercase();
    PROVIDERS.iter().find(|provider| {
        provider.id == normalized || provider.aliases.iter().any(|alias| *alias == normalized)
    })
}

pub fn normalize_base_url(base_url: &str) -> String {
    base_url.trim().trim_end_matches('/').to_ascii_lowercase()
}

pub fn infer_provider_from_base_url(base_url: &str) -> &'static str {
    let normalized = normalize_base_url(base_url);
    if normalized == "https://openrouter.ai/api/v1" {
        return "openrouter";
    }
    if normalized == "https://api.openai.com/v1" {
        return "openai";
    }
    if normalized == "https://api.groq.com/openai/v1" {
        return "groq";
    }
    if normalized == "https://api.x.ai/v1" {
        return "xai";
    }
    if normalized == "https://api.fireworks.ai/inference/v1" {
        return "fireworks";
    }
    if normalized == "https://integrate.api.nvidia.com/v1" {
        return "nvidia";
    }
    if normalized == "https://ollama.com/v1" {
        return "ollama-cloud";
    }
    if matches!(
        normalized.as_str(),
        "https://token-plan-sgp.xiaomimimo.com/v1"
            | "https://token-plan-sgp.xiaomimimo.com/anthropic"
            | "https://token-plan-cn.xiaomimimo.com/v1"
            | "https://token-plan-cn.xiaomimimo.com/anthropic"
            | "https://api.xiaomimimo.com/v1"
            | "https://api.xiaomimimo.com/anthropic"
    ) {
        return "xiaomi";
    }
    if let Some(local) = detect_local_model_server_url(&normalized) {
        return local;
    }
    "custom"
}

pub fn detect_local_model_server_url(base_url: &str) -> Option<&'static str> {
    let normalized = normalize_base_url(base_url);
    if normalized.contains("localhost:11434")
        || normalized.contains("127.0.0.1:11434")
        || normalized.contains("0.0.0.0:11434")
    {
        return Some("ollama");
    }
    if normalized.contains("localhost:1234")
        || normalized.contains("127.0.0.1:1234")
        || normalized.contains("0.0.0.0:1234")
    {
        return Some("lm-studio");
    }
    None
}
