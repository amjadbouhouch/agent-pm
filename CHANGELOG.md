# Changelog

All notable changes to this project will be documented in this file. See [commit-and-tag-version](https://github.com/absolute-version/commit-and-tag-version) for commit guidelines.

## [0.3.0](https://github.com/amjadbouhouch/agent-pm/compare/v0.2.0...v0.3.0) (2026-10-22)

### Features

* add claims with next, claim, release and claims ([c42339e](https://github.com/amjadbouhouch/agent-pm/commit/c42339e194e1300d69020e2fb1327054622f5b51))
* add export, import and backup commands ([9e0de08](https://github.com/amjadbouhouch/agent-pm/commit/9e0de0814fff09b8d8b6b3ad28b67f1c98fc6f3a))
* add install.ps1 Windows installer ([7e559e2](https://github.com/amjadbouhouch/agent-pm/commit/7e559e2806265d713389f08fb6b534fbded77fb4))

### Bug Fixes

* **claim:** address D1 review round 1 ([88768f1](https://github.com/amjadbouhouch/agent-pm/commit/88768f1cfefb2231c4a90471f91217f9b96c50bb))
* **db:** derive the event-kind list from the enum; plain error value ([2e3f592](https://github.com/amjadbouhouch/agent-pm/commit/2e3f592b5456e097f9b8ec18b5bffd49cb561545))
* **install.ps1:** address review round 1 ([2d85c65](https://github.com/amjadbouhouch/agent-pm/commit/2d85c65a06b1de193cb340099175adb3739c82b7))
* **port:** enforce command rules on import, harden backup and export ([6f61b88](https://github.com/amjadbouhouch/agent-pm/commit/6f61b8834cf491c7bbb96c45b3bf1f2a6aa7b1ef))
* **port:** reject unknown event kinds on import; test claims across export/import ([6acac77](https://github.com/amjadbouhouch/agent-pm/commit/6acac7738f55427c666479be61736c300412aad5))
* **show:** indent multi-line bodies and comment continuation lines ([25307d8](https://github.com/amjadbouhouch/agent-pm/commit/25307d81a1dbb118908c34368922a8557de57900))

### Documentation

* document the Windows installer ([9121131](https://github.com/amjadbouhouch/agent-pm/commit/91211310c945456ba3f7479f58f8125526b5c6c0))
* install.ps1 scriptblock form, env pinning, uninstall PATH ([3ce3d24](https://github.com/amjadbouhouch/agent-pm/commit/3ce3d24b203c68ac08e561383819735cdb6e21f7))
* reflow the backup paragraph in README ([ca8d16c](https://github.com/amjadbouhouch/agent-pm/commit/ca8d16c9a70c0312223eb025c6387b42f40055a9))
## [0.2.0](https://github.com/amjadbouhouch/agent-pm/compare/v0.1.0...v0.2.0) (2026-10-22)

### Features

* add task links, tags and history command ([9161879](https://github.com/amjadbouhouch/agent-pm/commit/91618790f6db67b7a59ef4139ad2ca1d1f753736))
* **item:** read bodies and comments from stdin or a file (APM-15) ([b8d3fca](https://github.com/amjadbouhouch/agent-pm/commit/b8d3fcab7805c6a60fb1fc9b3c87e9fd9b0065aa))
* **list:** add search, sort, limit, due filters and date-time values ([8aa3df5](https://github.com/amjadbouhouch/agent-pm/commit/8aa3df57c4849e6324a90839417b665887e4f482))
* merge backlog-1 (leaf progress, MS column, show --tree, stdin/file bodies, no-op edits) ([8286232](https://github.com/amjadbouhouch/agent-pm/commit/82862320f919c253702766435c5ff62c06ece2a4))
* merge v2-core (custom statuses, schema v2, history events) ([4735ec4](https://github.com/amjadbouhouch/agent-pm/commit/4735ec403e83816d7a3da4d19a98c673e4966876))
* merge v2-links (task links, tags, history) ([990f248](https://github.com/amjadbouhouch/agent-pm/commit/990f24813de77213ec6553bfe4d091c6d3be089a))
* merge v2-list (search, sort, limit, due filters, datetimes) ([8a34e1c](https://github.com/amjadbouhouch/agent-pm/commit/8a34e1c121368ee41e806ca5071f1b5e9f7941a4))
* merge v2-move (move tasks, re-key projects, rename attrs, aliases) ([af71ba0](https://github.com/amjadbouhouch/agent-pm/commit/af71ba03918c6a01ad19c6afbad2bef561cd938a))
* **move:** move tasks, re-key projects, rename attributes, resolve aliases ([23b0b10](https://github.com/amjadbouhouch/agent-pm/commit/23b0b108ab20f43b803f58adb8affb681a1aded6))
* **show:** --tree view, milestone column in list, aligned subtask lines (APM-12, APM-13) ([6138643](https://github.com/amjadbouhouch/agent-pm/commit/6138643cc2dc863e1c877f888166874c5e968253))
* **status:** schema v2 with custom statuses, history events and when parser ([3c5d138](https://github.com/amjadbouhouch/agent-pm/commit/3c5d138bb45e00db59bf3b075f9a84fc18e2c73a))

### Bug Fixes

* **db:** address v2-core review round 1 ([2efb919](https://github.com/amjadbouhouch/agent-pm/commit/2efb91921ce5a894bb65ed6c1a693306d9d14a39))
* **db:** merge fix-wal-retry (retry the WAL switch on a new db, APM-14) ([0fb5361](https://github.com/amjadbouhouch/agent-pm/commit/0fb53613a5d7add59b3b1ce4429aa3e135130c90))
* **db:** retry the WAL switch when another process holds a lock ([4842c41](https://github.com/amjadbouhouch/agent-pm/commit/4842c411eaeb4027128fa54ae13e33fb3487aa7b))
* **item:** address backlog-1 review round 1 ([9fe1f7a](https://github.com/amjadbouhouch/agent-pm/commit/9fe1f7ae89c4bc86ea0d7fcb908e6adfa203ca20))
* **item:** no-op edits keep updated_at and print unchanged (APM-16) ([764f8a6](https://github.com/amjadbouhouch/agent-pm/commit/764f8a68f73e4141d8b9c9cba104a9dccd308fc8))
* keep text output ASCII (history ->, +HH:MM/-HH:MM in help and errors) ([9298265](https://github.com/amjadbouhouch/agent-pm/commit/9298265198e21eb8f4d04a0b7824c5a79d5da46a))
* **link:** address B2 review round 1 ([3649f57](https://github.com/amjadbouhouch/agent-pm/commit/3649f5794a63bbdffcc0bde27313564baf3470c0))
* **list:** address B1 review round 1 ([948eeab](https://github.com/amjadbouhouch/agent-pm/commit/948eeabaeb77183957bd09553fb45edb3248a050))
* **milestone:** count leaf tasks only in progress (APM-11) ([b350d57](https://github.com/amjadbouhouch/agent-pm/commit/b350d576943dee8cff47cae78ceeb776f1d1fa79))
* **move:** address B3 review round 1 ([9c3a9da](https://github.com/amjadbouhouch/agent-pm/commit/9c3a9daf227d5b1213622b9319bf7d3a4877a2b3))

### Refactoring

* **db:** share current_key for a task's current KEY-N ([7b77c58](https://github.com/amjadbouhouch/agent-pm/commit/7b77c581203910d9807c29ab0f120f67611d77b3))

### Documentation

* correct rename wording in README, tags/links/history in AGENTS.md ([4059c86](https://github.com/amjadbouhouch/agent-pm/commit/4059c866eb468c6cabad1cb2e8cfe3c8b44d3ad4))
* v2 features and concepts in README, AGENTS.md layout and conventions, guide ([24fc51e](https://github.com/amjadbouhouch/agent-pm/commit/24fc51ea87616efb1d7bf118d88bdb12971bad5c))
## 0.1.0 (2026-10-22)

### Bug Fixes

* address s6 review round 1 ([a94271f](https://github.com/amjadbouhouch/agent-pm/commit/a94271fe06b90bf37ba05bbbe67361d47df15083))

### Documentation

* add initial README ([dcb30af](https://github.com/amjadbouhouch/agent-pm/commit/dcb30af6604ef298b7f76889f3c7693dbff4bdb1))
# Changelog

All notable changes to this project will be documented in this file. See [commit-and-tag-version](https://github.com/absolute-version/commit-and-tag-version) for commit guidelines.
