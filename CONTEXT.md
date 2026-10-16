# AI SubHub

A hub for seeing remaining AI subscription capacity and opening tools in the intended account and project.

## Language

**Provider**: The AI service whose subscription allowance is shown: Claude or Codex in the first version.

**Account**: A named connection to one provider login. Its name is an editable label chosen by the user, not proof of the provider identity.
_Avoid_: Subscription as a synonym for the editable account entry.

**Profile**: The provider-specific workspace containing a login and its associated settings and history. An account refers to one profile; removing the account leaves the profile intact.

**Allowance window**: A provider-defined period or category of usage with its own remaining capacity and reset time. An account can have several windows.
_Avoid_: Balance, which suggests purchased credit.

**Reading**: An observation of an account's allowance windows at a known time.

**Stale reading**: A previously successful reading whose refresh failed or is overdue. It is last known usage, not a current promise.

**Project folder**: A saved working folder used when launching an account's tool. The list is shared by all accounts.

**Visible project**: A saved project folder currently included in Launch menus, in the user's chosen order.

**Hidden project**: A saved project folder temporarily excluded from Launch menus. Hiding preserves its identity, folder and position so it can be shown again.

**Claude chat Project**: A workspace in Claude containing chats, instructions and knowledge files. It has its own identity and access rules, separate from a local project folder.
