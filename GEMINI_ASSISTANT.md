# Note for Claude and GPT5: Using Gemini CLI as an Assistant

Gemini is an AI model by Google with deep understanding of large contexts at a very affordable cost. This document describes how to use `gemini` (gemini-cli) as an assistant for specific tasks.

## Installation

Gemini CLI is already installed and available via the `gemini` command.

Current version: `0.18.4`

## Basic Usage

### Sandbox Mode (Recommended)

**Always use sandbox mode** to prevent unintended modifications:

```bash
# One-shot query with sandbox enabled
gemini -s "your question here"

# Interactive mode with sandbox
gemini -s -i "initial prompt"
```

### Command Reference

| Flag | Description |
|------|-------------|
| `-s, --sandbox` | **Required for safety** - Run in sandbox mode |
| `-m, --model` | Specify model to use |
| `-i, --prompt-interactive` | Start interactive session with initial prompt |
| `-y, --yolo` | Auto-accept all actions (⚠️ dangerous) |
| `--approval-mode` | Set approval: `default`, `auto_edit`, or `yolo` |
| `-r, --resume` | Resume previous session (`latest` or index number) |
| `--list-sessions` | List available sessions |

## Recommended Use Cases

Gemini excels at tasks involving **large context** and **simple but tedious work**:

### 1. Summarizing Large Files

```bash
# Summarize a large source file (use -p with stdin, or just positional arg)
cat path/to/large_file.rs | gemini -s -p "Summarize the main functionality and public API of this file"

# Alternative: pass file content inline
gemini -s "Summarize this code: $(cat path/to/large_file.rs)"

# Get an overview of multiple files
gemini -s "Explain the architecture: $(find ./src -name '*.rs' -exec echo '=== {} ===' \; -exec head -50 {} \;)"
```

> **Note:** When using stdin (`|` or `<`), use the `-p` flag for the prompt text. Don't combine stdin with positional prompts.

### 2. Reading and Understanding Code

```bash
# Understand a complex function
gemini -s "Explain what this code does step by step"

# Find patterns in codebase
gemini -s "List all the error handling patterns used in this file"
```

### 3. Simple Repetitive Tasks

```bash
# Generate boilerplate
gemini -s "Generate test cases for these functions"

# Format or transform data
gemini -s "Convert this JSON to TOML format"
```

### 4. Documentation Tasks

```bash
# Generate documentation
gemini -s "Generate rustdoc comments for these public functions"

# Review and improve docs
gemini -s "Review this README and suggest improvements"
```

## ⚠️ Important Limitations

**Gemini may make mistakes.** Always verify its output for:

- Code correctness
- Logic errors
- Syntax issues
- Context misunderstanding

### Best Practices

1. **Always use sandbox mode** (`-s`) unless you absolutely need file modifications
2. **Review all output** before applying changes
3. **Use for read-only tasks** when possible (summarization, explanation)
4. **Break complex tasks** into smaller, simpler steps
5. **Provide clear, specific prompts** - Gemini works better with explicit instructions

## Example Workflows

### Reviewing a Large PR

```bash
# Get a summary of changes
git diff main...feature-branch | gemini -s -p "Summarize these changes and identify potential issues"
```

### Understanding Unfamiliar Code

```bash
# Explain a module's purpose
cat src/complex_module.rs | gemini -s -p "Explain the purpose and design of this module"
```

### Generating Repetitive Code

```bash
# Generate FFI bindings (review output carefully!)
cat src/lib.rs | gemini -s -p "Generate C FFI function signatures for these Rust functions"
```

## Subagent Manager

A `subagent` script is provided to elegantly manage background Gemini tasks:

```bash
# Make executable (already done)
chmod +x ./subagent
```

### Quick Start

```bash
# Launch a subagent
./subagent run bugfind "Find bugs in this code"

# Launch with file content
./subagent run-file review ./src/lib.rs "Review this code"

# Launch analyzing a directory
./subagent run-dir android-bugs ./backends/android "*.kt" "Find bugs in Android backend"

# Check status of all subagents
./subagent status

# Wait for a specific subagent
./subagent wait bugfind

# Get results
./subagent result bugfind
```

### All Commands

| Command | Description |
|---------|-------------|
| `run <name> <prompt>` | Launch subagent with prompt |
| `run-file <name> <file> <prompt>` | Launch with file as context |
| `run-dir <name> <dir> <glob> <prompt>` | Launch analyzing directory |
| `status [name]` | Show status (all or specific) |
| `wait <name>` | Wait for specific subagent |
| `wait-any` | Wait for any to complete |
| `wait-all` | Wait for all to complete |
| `result <name>` | Show completed output |
| `tail <name>` | Live tail running output |
| `kill <name>` | Kill a running subagent |
| `clean` | Remove completed subagent data |
| `list` | List all subagents |

### Workflow Example

```bash
# Launch multiple analysis tasks in parallel
./subagent run-dir bug-android ./backends/android "*.kt" "Find bugs"
./subagent run-dir bug-rust ./src "*.rs" "Find memory safety issues"
./subagent run-dir doc-review ./docs "*.md" "Review documentation"

# Do other work while they run...

# Check status
./subagent status

# Wait for all to complete
./subagent wait-all

# Review results
./subagent result bug-android
./subagent result bug-rust
./subagent result doc-review

# Clean up
./subagent clean
```

### Data Location

Subagent data is stored in `~/.subagents/<name>/`:
- `prompt` - The original prompt
- `output.txt` - Gemini's response
- `pid` - Process ID
- `started` / `finished` - Timestamps
- `exit_code` - Exit status

## Integration with Cursor

When using Cursor AI, delegate to Gemini for:
- Reading and summarizing very large files (>1000 lines)
- Understanding legacy code with complex patterns
- Generating initial drafts of repetitive code
- Getting quick explanations of unfamiliar code sections
- **Background code analysis** (run as subagent)

Keep in Cursor:
- Actual code editing and modifications
- Complex reasoning and architecture decisions
- Tasks requiring high accuracy
- Interactive debugging and problem-solving

