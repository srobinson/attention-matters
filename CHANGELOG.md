# Changelog

## [0.1.4](https://github.com/srobinson/attention-matters/compare/v0.1.3...v0.1.4) (2026-02-21)


### Features

* AM plugin for Claude Code — zero-config persistent memory (ALP-552) ([#10](https://github.com/srobinson/attention-matters/issues/10)) ([b8040f4](https://github.com/srobinson/attention-matters/commit/b8040f4d1a5fa2c6f6fe4a5f8dda06bf5dfff4e1))
* decisions stick — typed neighborhoods, session dedup, decision-aware scoring ([f1f63fd](https://github.com/srobinson/attention-matters/commit/f1f63fd573eb7b803d0d56b00b915432a096776d))
* graceful shutdown — am serve exits cleanly on stdin EOF and signals (ALP-573) ([#11](https://github.com/srobinson/attention-matters/issues/11)) ([5822798](https://github.com/srobinson/attention-matters/commit/582279805a07c678415a4c45ded2a4dfaf04a276))
* one brain per developer — unified brain.db replaces per-project stores ([82f7e59](https://github.com/srobinson/attention-matters/commit/82f7e5968f0c03eeea7487aca3c224ddc6fcdab1))
* recency-aware memory recall — timestamps, backfill, conscious boost ([ad69e92](https://github.com/srobinson/attention-matters/commit/ad69e9289df307da14e3d4fef55b9d909a502d02))
* self-healing memory layer (ALP-569) ([#8](https://github.com/srobinson/attention-matters/issues/8)) ([81a153b](https://github.com/srobinson/attention-matters/commit/81a153b2eb45faa1db0cd50cafa21fec8b599191))
* surface recalled neighborhood IDs for feedback loop ([022a4fb](https://github.com/srobinson/attention-matters/commit/022a4fb79bf9048fdb44add6b2005fc4f8f02f61))
* unified brain — remove per-project concept, simplify APIs ([28a275f](https://github.com/srobinson/attention-matters/commit/28a275f2b45ae1b4cbac4e5afd43f9a9b7fb5a7e))
* world-class CLI — inspect, help, sync, gc, forget (ALP-557) ([#9](https://github.com/srobinson/attention-matters/issues/9)) ([5e7e6c5](https://github.com/srobinson/attention-matters/commit/5e7e6c571ec467edfbe16414cff235ebd912a01e))


### Bug Fixes

* add recalled_ids to am_batch_query response ([6255ecd](https://github.com/srobinson/attention-matters/commit/6255ecd2fe57be66116c550f6ec668cfeea80a97))
* npm binary collision — move native binary to scripts/ ([3776316](https://github.com/srobinson/attention-matters/commit/3776316bc4a950e6b8c5c1f69d7032d98b710dcd))
* prevent silent data destruction on failed system load ([7a229aa](https://github.com/srobinson/attention-matters/commit/7a229aaa2332989e4a1fa4288b1871d13b5ffc69))
* reduce buffer threshold to 3 and flush orphaned buffers on query ([f87961e](https://github.com/srobinson/attention-matters/commit/f87961e0754557d6136267da7fdf03662a33ae4f))
* remove [@alphab](https://github.com/alphab).io/am from release pipeline ([9c0b67c](https://github.com/srobinson/attention-matters/commit/9c0b67ca72df86367a0d6a2a6cda56d564aeb4f0))

## [0.1.3](https://github.com/srobinson/attention-matters/compare/v0.1.2...v0.1.3) (2026-02-14)


### Bug Fixes

* idempotent npm publish and remove invalid bin entries ([13509ed](https://github.com/srobinson/attention-matters/commit/13509ed34fd8d3ecec277cce144da9b568438c6e))

## [0.1.2](https://github.com/srobinson/attention-matters/compare/v0.1.1...v0.1.2) (2026-02-14)


### Bug Fixes

* use macos-latest for x86_64 darwin builds ([2eefbc4](https://github.com/srobinson/attention-matters/commit/2eefbc4d26f1a7f81cc5b608938f4e26a4303c2b))

## [0.1.1](https://github.com/srobinson/attention-matters/compare/v0.1.0...v0.1.1) (2026-02-14)


### Features

* **am-store:** SQLite persistence layer ([8a393c2](https://github.com/srobinson/attention-matters/commit/8a393c2ff3f4fc2b901f9bc7ea7f14a79fa8d8c6))
* automate releases with release-please ([5459797](https://github.com/srobinson/attention-matters/commit/54597978a9e68c06ca3dbaa58a5014ca10310319))


### Bug Fixes

* drop cargo-workspace plugin from release-please ([93fdcc9](https://github.com/srobinson/attention-matters/commit/93fdcc9a95cc502c763630dbc82c76063953fb29))
* remove NPM_TOKEN secret, use pure OIDC trusted publishing ([06b8789](https://github.com/srobinson/attention-matters/commit/06b87899a49a6ef6f59e2836fe72d7e79b679d79))
* use simple release type and add CI workflow ([f485e9c](https://github.com/srobinson/attention-matters/commit/f485e9c798c231ba2e18b6a2555426e37b10fa2a))
