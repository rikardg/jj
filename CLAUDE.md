# Version Control

This project uses **jj (Jujutsu)** for version control, not git. Use `jj` commands instead of `git` for all VCS operations.

## Essential Commands

| Task | Command |
|------|---------|
| Show status | `jj status` or `jj st` |
| Show diff | `jj diff` (working copy) or `jj diff -r <rev>` |
| Show log | `jj log` |
| Create new commit | `jj new` (creates empty child of current) |
| Finalize working copy | `jj commit -m "message"` |
| Edit commit message | `jj describe -m "message"` |
| Create bookmark (branch) | `jj bookmark set <name>` |
| Push to remote | `jj git push` |

## Key Differences from Git

- **No staging area.** All changes in the working copy are automatically tracked.
- **Working copy is a commit.** The `@` symbol refers to the current working-copy commit.
- **`jj new` instead of branching.** Creates a new empty commit to work on.
- **`jj commit` = finalize + create next.** It finalizes the current working-copy commit and creates a new empty one on top.
- **Bookmarks, not branches.** `jj bookmark set feature-x` is equivalent to `git branch feature-x`.

## Splitting Commits with `jj split`

`jj split` divides a commit into two. It's the primary tool for organizing changes into clean, focused commits.

### Non-interactive splitting (preferred for agents)

**By files** — put specific files into the first commit:
```sh
jj split file1.rs file2.rs -m "first commit message"
```

**By diff** — apply a unified diff to select exact hunks:
```sh
# Generate a diff of just the changes you want, then pipe it in
jj diff --git -r @ | jj split --diff - -m "selected changes"

# Or from a patch file
jj split --diff changes.patch -m "selected changes"
```

The `--diff` flag accepts standard git unified diff format. Changes in the diff go into the first commit; everything else goes into the second.

### Placement options

By default, split creates parent (selected) → child (remaining). Other options:

- `--parallel` / `-p` — make sibling commits instead of parent-child
- `--onto <rev>` / `-o` — extract selected changes onto a different revision
- `--insert-before <rev>` / `-B` — insert the selected changes before a revision
- `--insert-after <rev>` / `-A` — insert the selected changes after a revision

### Typical workflow: break up a large change

```sh
# Split by files into focused commits
jj split src/parser.rs src/lexer.rs -m "refactor: extract parser module"
# Remaining changes are still in @
jj describe -m "feat: add new syntax support"
```

## Moving Changes with `jj squash`

`jj squash` moves changes from one commit into another (by default, from working copy into its parent).

### Non-interactive squashing (preferred for agents)

**By files** — move specific files:
```sh
jj squash file1.rs file2.rs
```

**By diff** — apply a unified diff to select exact hunks to move:
```sh
# Squash specific hunks from working copy into parent
jj diff --git | jj squash --diff -

# From a patch file
jj squash --diff changes.patch
```

**Between specific revisions:**
```sh
# Move changes from revision X into revision Y
jj squash --from X --into Y

# Move working copy changes into grandparent
jj squash --into @--
```
