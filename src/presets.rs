//! Ready-made vocabularies. They list what whisper gets wrong (identifiers, brands, tools), not
//! plain English like "commit" or "branch", which it already spells and which would only make
//! sound matching more likely to misfire.

pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub terms: &'static [&'static str],
}

pub const PRESETS: &[Preset] = &[
    Preset {
        id: "linux",
        name: "Linux & shell",
        terms: &[
            "sudo", "pacman", "yay", "systemctl", "journalctl", "chmod", "chown", "grep", "ripgrep",
            "sed", "awk", "tmux", "rsync", "curl", "wget", "Hyprland", "Wayland", "Neovim", "zsh",
            "fish", "dotfiles", "stdout", "stderr", "symlink", "SSH", "fstab", "btrfs", "PipeWire",
        ],
    },
    Preset {
        id: "web",
        name: "Web development",
        terms: &[
            "JavaScript", "TypeScript", "useEffect", "useState", "useMemo", "Next.js", "Node.js",
            "npm", "pnpm", "Bun", "Vite", "Tailwind", "JSON", "GraphQL", "ESLint", "Svelte",
            "Vue", "localhost", "CORS", "WebSocket", "OAuth", "JWT", "HTML", "CSS", "DOM",
        ],
    },
    Preset {
        id: "rust",
        name: "Rust",
        terms: &[
            "rustc", "cargo", "rustup", "tokio", "serde", "clippy", "rustfmt", "impl", "enum",
            "async", "Vec", "HashMap", "Arc", "Mutex", "RefCell", "unwrap", "dyn", "wasm",
        ],
    },
    Preset {
        id: "python",
        name: "Python",
        terms: &[
            "NumPy", "pandas", "PyTorch", "Jupyter", "pytest", "FastAPI", "Django", "Flask",
            "venv", "pip", "uv", "asyncio", "kwargs", "dataclass", "Pydantic", "matplotlib",
        ],
    },
    Preset {
        id: "devops",
        name: "DevOps & cloud",
        terms: &[
            "Docker", "Kubernetes", "kubectl", "Helm", "Terraform", "AWS", "GCP", "Azure",
            "nginx", "PostgreSQL", "Redis", "SQLite", "YAML", "Grafana", "Prometheus", "CI/CD",
            "GitHub", "GitLab", "Cloudflare", "DNS",
        ],
    },
    Preset {
        id: "ai",
        name: "AI & ML",
        terms: &[
            "LLM", "GPT", "Claude", "Anthropic", "OpenAI", "embeddings", "tokenizer", "RAG",
            "CUDA", "Whisper", "Hugging Face", "LoRA", "MCP", "Ollama", "llama.cpp", "GGUF",
        ],
    },
];

pub fn find(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}
