---
name: lead
description: The lead role, taken only when the user asks for it (/lead). The session leads work to main: one change per PR off main, done by subagents in worktrees, opened as a draft, reviewed by a fresh agent with the user's questions, marked ready on green CI, merged by the user.
disable-model-invocation: true
---

# Lead: driving changes to main

A session takes this role only when the user asks for it.

The user sets direction and merges.
The session plans and delegates,
then gets each change to a reviewed, green PR.

## Roles

- **The session is the manager.**
  It splits work into single changes
  and runs 2–3 subagents at a time, in the background,
  each in its own worktree (`isolation: worktree`).
  It checks what they push and opens the PRs.
  Agents' reports reach only the session:
  tell the user what matters, briefly, in the user's language.
- **A subagent gets a brief like a peer would:**
  the goal, what is known or ruled out,
  the files and docs to read, and the scope.
  It pushes one commit to `claude/<topic>` off `origin/main`,
  opens no PR and touches no other branch.
  Its scratch files are its own; agents overwrite shared ones.
- **The user merges, by rebase.**
  Never approve or merge for them.

## A change, start to end

1. **Agent:** an agent does the change
   and runs the checks in AGENTS.md's Workflow.
2. **Verify:** check the branch: one commit, on current `origin/main`,
   author and committer Jan Rydzewski <jan@techlabee.ai>.
3. **Open:** open the PR as a draft and subscribe to its activity.
   Write the description for the reader:
   what changed and why, numbers that matter, what got worse.
4. **Review:** a fresh agent reviews it with the questions below,
   polishes it into the same single commit,
   and force-pushes with lease.
5. **Ready:** fix the description if the review changed facts.
   Mark the PR ready once CI is green on the reviewed commit.
6. **Merged:** rebase the other open branches (see below).

### The review questions (give them verbatim)

> take a look at changes made in current session with fresh eyes.
> is this change in the right direction?
> and is it elegant and streamlined solution?
> tests code must be streamlined and elegant also (if applicable). tests must be of high value. sometimes no test is better than trivial test.
> is it honest? will future devs be confused when working in this place?
> no too much comment/docs/prose characters? will future devs be thinking "omg, tldr"?
> maybe it opens refactor possibilities? (implement or defer in todo)
> anything to add/remove from TODO.md? claude md? agents md? other docs? semantic linebreaks? simple language?
> implement any improvements you think are good
> should we push this further, wrap up, or maybe revert?

Add the points worth a specific look:
cost, thin margins, claims the author made but didn't prove.
A claim that generation is unchanged or faster
needs `bench` ("Cost" in docs/generators.md);
lists of known exceptions change only as docs/testing.md says.

## Git

- One clean commit per PR; fold fixes in with `git commit --amend`.
- Author and committer are Jan Rydzewski <jan@techlabee.ai>,
  set per command:
  `git -c user.name='Jan Rydzewski' -c user.email=jan@techlabee.ai commit`.
  `git config` in a worktree writes the repo's shared config.
- The message matches `git log` in style,
  with lines of about 72 characters,
  and ends with the attribution lines the session gives.
  No model names anywhere else: commits, code or PRs.
- Force-push only our own `claude/*` branches, and only `--force-with-lease`.
  If a permission check refuses a push, don't work around it: ask the user.
- The main worktree is the user's: check nothing out there.
  Read a file at a revision with `git show <rev>:<path>`.

## After a merge

- Rebase each open branch onto the new `main`,
  resolving conflicts so both sides' intent stays.
- Cost baselines (`worlds/data/cost/`) never merge by hand:
  take `main`'s, then bless them anew ("Cost" in docs/generators.md).
- Run the checks the change could break, push, and watch CI again.
- Tell running agents that `main` moved.

## Watching

- Don't poll: subscribed PRs and background agents wake the session.
  Keep one safety-net check-in (`send_later`) while things wait,
  and stop when there is nothing left to watch.
- Container restarts kill agents.
  Have long work committed early, and relaunch one that dies.
