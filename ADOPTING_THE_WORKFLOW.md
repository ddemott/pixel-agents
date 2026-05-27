# Adopting the Portable Development Workflow

**This is the single document you need.**

If someone sent you here (especially from a sibling/parallel project like `../ai-sec`), follow the instructions in this file exactly. You should not need any additional files or explanations from the person who pointed you at it.

The goal is to give your project the same battle-tested system for:

- Consistent feature branching
- Automated quality gates before committing
- Documentation hygiene
- Reliable commit and PR processes

Everything is driven from one configuration file (`workflow.config.json`). The automation scripts adapt based on your declared `projectType`.

---

## Quick Start (When ai-sec is a Sibling Directory)

This is the most common case when working alongside the source repo.

From the root of **your** project, the easiest way is to grab the pre-built kit that has already been dropped at the parent level:

```bash
# Recommended for SaaS / TypeScript projects (pre-specialized for node-fullstack)
cp ../portable-workflow-kit-node-fullstack-2026-05-27.zip .
unzip portable-workflow-kit-node-fullstack-2026-05-27.zip

# Or grab the latest generic version
cp ../portable-workflow-kit-2026-05-27.zip .
unzip portable-workflow-kit-2026-05-27.zip
```

You can also use the always-up-to-date folders:

- `../portable-workflow-kit-node-fullstack-latest/` (best for SaaS projects)
- `../portable-workflow-kit-latest/`

After unzipping or copying a folder, you will have a `portable-workflow-kit/` directory ready to use.

**Also copy this document** if you want it locally:

```bash
cp ../ai-sec/docs/ADOPTING_THE_WORKFLOW.md .
```

### Alternative: Ask maintainer to generate a fresh one

If you want the absolute latest version, ask them to run this helper (it keeps the sibling kits fresh):

```bash
cd ../ai-sec
bash scripts/refresh-workflow-kits.sh
```

This updates both the generic and node-fullstack versions in the parent directory.

Manual generation (if needed):

```bash
cd ../ai-sec
npm run generate-kit -- --zip --project-type node-fullstack   # SaaS recommended
# or
npm run generate-kit -- --zip --project-type python
```

---

## Step-by-Step (Do These in Order)

### 1. Choose Your Project Type (Most Important Step)

Open `portable-workflow-kit/workflow.config.json` and set the `projectType` field near the top:

```json
"projectType": "python"     // or "node-fullstack", "generic"
```

**Available types** (see `projectTypeProfiles` in the same file):

- `node-fullstack` — TypeScript SaaS projects (Next.js + backend)
- `python` — Python projects (FastAPI, Django, CLIs, etc.) — uses ruff, black, pytest
- `generic` — Everything else (Go, Rust, mixed, etc.)

The scripts will automatically use only the commands appropriate for your type. A Python project will never be told to run `eslint` or `tsc`.

### 2. Customize the Commands for Your Project

Still in `workflow.config.json`, look at the `commands` section.

Copy the matching block from `projectTypeProfiles` into the active `commands` object, then edit the strings to match your actual tools.

Example for a Python project:

```json
"commands": {
  "checks": "ruff check . && black --check .",
  "unitTests": "pytest -q --tb=line",
  "build": "python -m py_compile $(git ls-files '*.py') 2>/dev/null || true",
  "lint": "ruff check .",
  "formatCheck": "black --check .",
  "docDriftCheck": "echo 'Add your doc drift command here or leave empty'",
  ...
}
```

This is the **only file** most projects need to edit significantly.

### 3. Install the Scripts

Copy the contents of `portable-workflow-kit/scripts/` into a `scripts/` folder in your project root (create the folder if needed).

Make them executable:

```bash
chmod +x scripts/*.sh
```

### 4. Add Convenience Scripts (Optional but Recommended)

Add these to your `package.json` (if you use npm) or equivalent task runner:

```json
"scripts": {
  "create-branch": "bash scripts/create-feature-branch.sh",
  "prepare-commit": "bash scripts/prepare-commit.sh",
  "setup-hooks": "bash scripts/setup-hooks.sh",
  "remove-hooks": "bash scripts/remove-hooks.sh"
}
```

For non-Node projects, you can run the scripts directly:

```bash
bash scripts/create-feature-branch.sh feat/my-feature
bash scripts/prepare-commit.sh
```

### 5. Set Up Git Hooks (Strongly Recommended)

The system works best with Husky for automatic hook installation.

1. Install Husky: `npm install --save-dev husky` (or equivalent)
2. Add `"prepare": "husky"` to your package.json scripts.
3. Copy the `.husky/` examples from the kit (or run the setup scripts).

After `npm install`, the pre-commit and pre-push hooks will be active.

### 6. Copy Supporting Files

From the kit, copy these into your project:

- `BRANCH_CHECKLIST.md` → put a copy at your project root when starting new work
- `.github/pull_request_template.md`
- `.github/BRANCH_PROTECTION.md`
- `.github/ISSUE_TEMPLATE/` (optional but useful)

### 7. Start Using It

- Always create branches with: `npm run create-branch feat/your-name` (or the script directly)
- Before committing significant work, run: `npm run prepare-commit`
- Update the files listed in your `workflow.config.json` under `documentation.filesThatMustBeUpdated`

---

## Staying Up to Date

When the source project improves the workflow:

1. Ask the maintainer to run `npm run generate-kit -- --zip --project-type <your-type>` (or whatever the latest recommended command is).
2. Replace your local `portable-workflow-kit/` with the new generated version.
3. Re-apply any customizations you made to `workflow.config.json`.

---

## Common Questions

**Q: Do I have to use npm?**  
No. The scripts are plain bash and work on any project. Use them directly or wire them into Make, Poe, Task, etc.

**Q: What if my tooling is different?**  
Edit the `commands` section in `workflow.config.json`. The automation will run whatever strings you put there (or skip them gracefully if left as `echo` placeholders).

**Q: Where is the full philosophy and history?**  
See the root `ADOPTING_THE_WORKFLOW.md` and `PORTABLE_DEVELOPMENT_WORKFLOW.md` in the ai-sec repo for deeper background. This document is intentionally the practical "just get it working" version.

---

**Welcome to a more disciplined way of building software.**

Once you have followed the steps above, you have the same system. The only ongoing work is keeping your `workflow.config.json` accurate for your project.
