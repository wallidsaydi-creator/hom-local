# Configuration

## Overview

HOM Local uses a layered configuration system with environment variables, configuration files, and command-line arguments.

## Configuration locations

1. **Environment variables**: `HOM_*` prefix
2. **Config file**: `~/.hom/config.json`
3. **Command line**: `hom-brain --flag value`

## Environment variables

| Variable | Description | Default |
|----------|-------------|---------|
| `HOM_DIR` | Home directory for HOM data | `~/.hom` |
| `HOM_BRAIN_SOCK` | Brain socket path | `~/.hom/brain.sock` |
| `HOM_LOG_LEVEL` | Log level (debug, info, warn, error) | `info` |
| `HOM_SQLITE_BUSY_TIMEOUT` | SQLite busy timeout (ms) | `5000` |
| `HOM_SQLITE_WAL_CHECKPOINT` | WAL checkpoint mode | `passive` |

## Config file format

```json
{
  "brain": {
    "socket_path": "~/.hom/brain.sock",
    "data_dir": "~/.hom/data",
    "log_level": "info"
  },
  "sqlite": {
    "busy_timeout_ms": 5000,
    "wal_checkpoint": "passive",
    "cache_size": -8000
  },
  "permissions": {
    "profile": "sandbox",
    "automation_enabled": false
  }
}
```

## Permission profiles

| Profile | Description |
|---------|-------------|
| `sandbox` | Default — restricted access |
| `full_access` | Full access with automation |
| `full` | Unrestricted access |

## Provider configuration

### OpenAI-compatible

```json
{
  "providers": {
    "openai-compat": {
      "enabled": true,
      "base_url": "https://api.openai.com/v1",
      "default_model": "gpt-4"
    }
  }
}
```

### Anthropic

```json
{
  "providers": {
    "anthropic": {
      "enabled": true,
      "default_model": "claude-sonnet-4-20250514"
    }
  }
}
```

### Local models (Ollama)

```json
{
  "providers": {
    "local-models": {
      "enabled": true,
      "base_url": "http://localhost:11434/v1",
      "default_model": "llama3"
    }
  }
}
```

## Logging

HOM Local uses the `tracing` framework with structured logging:

```bash
# Debug logging
RUST_LOG=debug hom-brain

# Filter specific modules
RUST_LOG=hom_brain=debug,hom_shared=info hom-brain

# JSON logging
RUST_LOG=json hom-brain
```
