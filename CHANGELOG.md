# Changelog

## [0.2.0](https://github.com/Malanius/pidjezdy/compare/v0.1.0...v0.2.0) (2026-09-12)


### Features

* install zsh completions with just ([#92](https://github.com/Malanius/pidjezdy/issues/92)) ([bc22feb](https://github.com/Malanius/pidjezdy/commit/bc22febb4d30c45c88fbe0091bb989ff99bf0ac8))

## 0.1.0 (2026-09-09)


### Features

* **cli:** add cache management commands ([#56](https://github.com/Malanius/pidjezdy/issues/56)) ([0482c4a](https://github.com/Malanius/pidjezdy/commit/0482c4a58b8230139f04e151e25b580df14057ab))
* **cli:** add labeled stale cache fallback ([#5](https://github.com/Malanius/pidjezdy/issues/5)) ([c44c3e2](https://github.com/Malanius/pidjezdy/commit/c44c3e27633adb7c81fccb1e7e2246ef36df9497))
* **cli:** add shell completion generation ([#11](https://github.com/Malanius/pidjezdy/issues/11)) ([321be3f](https://github.com/Malanius/pidjezdy/commit/321be3fcb4411f5ffb93f8453c1074ac4063b4ad))
* **cli:** emit versioned JSON error envelopes ([#51](https://github.com/Malanius/pidjezdy/issues/51)) ([241cd29](https://github.com/Malanius/pidjezdy/commit/241cd29b61c097910812d3fb79d7ea219e980542))
* **cli:** redesign human-readable departure output ([#12](https://github.com/Malanius/pidjezdy/issues/12)) ([0a103a2](https://github.com/Malanius/pidjezdy/commit/0a103a285d37f761d80b49581996c0de46f49514))
* **cli:** show configured reachable departures ([#4](https://github.com/Malanius/pidjezdy/issues/4)) ([edb6ddb](https://github.com/Malanius/pidjezdy/commit/edb6ddb19c838332de928a0ff6ffddf19ed5f501))
* **cli:** warn when PID truncates departure groups ([#58](https://github.com/Malanius/pidjezdy/issues/58)) ([82ded73](https://github.com/Malanius/pidjezdy/commit/82ded738e6f4841ee2c8b11e938d95a60973c77e))
* **core:** add per-route departure minimums ([#9](https://github.com/Malanius/pidjezdy/issues/9)) ([e9d4729](https://github.com/Malanius/pidjezdy/commit/e9d4729a8bd1a2340494632d800895d438e00eb6))
* **core:** add reachable departure ranking ([#2](https://github.com/Malanius/pidjezdy/issues/2)) ([9ecd902](https://github.com/Malanius/pidjezdy/commit/9ecd902f4b50e1430de5ff1b19e72be1965c8bf7))
* establish workspace and configuration foundation ([#1](https://github.com/Malanius/pidjezdy/issues/1)) ([c30e485](https://github.com/Malanius/pidjezdy/commit/c30e485047f32bf53a1872bcd838630825946ce1))
* humanise stale age and surface departure delay ([#61](https://github.com/Malanius/pidjezdy/issues/61)) ([69480a8](https://github.com/Malanius/pidjezdy/commit/69480a8417ef80f08e47c8a3e48236f7ea29ca91))
* **pid:** add defensive departure API adapter ([#3](https://github.com/Malanius/pidjezdy/issues/3)) ([cd270e5](https://github.com/Malanius/pidjezdy/commit/cd270e5096ace9b322662f80e9a1fc6ca199f066))
* **plugin:** add Omarchy departures widget ([#10](https://github.com/Malanius/pidjezdy/issues/10)) ([39d34af](https://github.com/Malanius/pidjezdy/commit/39d34af316582e5058abbd9375e322c5a81cf506))
* **plugin:** make the CLI binary path configurable ([#59](https://github.com/Malanius/pidjezdy/issues/59)) ([46d5dc0](https://github.com/Malanius/pidjezdy/commit/46d5dc05b8b108b787004a010633004519b4cf65))
* report relevant cancelled departures ([#60](https://github.com/Malanius/pidjezdy/issues/60)) ([3119561](https://github.com/Malanius/pidjezdy/commit/311956179ea76fed618bef260d4149c1c125b5a6))
* **ui:** pair clock times with countdowns ([#84](https://github.com/Malanius/pidjezdy/issues/84)) ([d60cff3](https://github.com/Malanius/pidjezdy/commit/d60cff39bb68fcbb9363a0ae0ec1f369ca627786))


### Bug Fixes

* **cli:** improve cache diagnostics and coverage ([#41](https://github.com/Malanius/pidjezdy/issues/41)) ([b005d1c](https://github.com/Malanius/pidjezdy/commit/b005d1c35f428accd3ca0edff54f60fac428ebfb))
* **cli:** preserve and render error causes ([#50](https://github.com/Malanius/pidjezdy/issues/50)) ([4fdda10](https://github.com/Malanius/pidjezdy/commit/4fdda104327f7e7fa66403da081d79cc6fcadcb5))
* **cli:** preserve cache on empty PID response ([#53](https://github.com/Malanius/pidjezdy/issues/53)) ([966b6b0](https://github.com/Malanius/pidjezdy/commit/966b6b0b363c1ad7e402596c0bf958e4ad95c34f))
* **core:** cap cancellation notes at departure limit ([#73](https://github.com/Malanius/pidjezdy/issues/73)) ([c1738b1](https://github.com/Malanius/pidjezdy/commit/c1738b10d26d76e9723c2e359d1408347205cb11))
* **core:** reject unnormalized programmatic configs ([#76](https://github.com/Malanius/pidjezdy/issues/76)) ([31593d5](https://github.com/Malanius/pidjezdy/commit/31593d5b708bf7f878edf4c46365b14604291a7d))
* **pid:** harden HTTP request handling ([#39](https://github.com/Malanius/pidjezdy/issues/39)) ([3b1031d](https://github.com/Malanius/pidjezdy/commit/3b1031d22d373de2b530e740d9e13db85543223c))
* **pid:** preserve response parse diagnostics ([#34](https://github.com/Malanius/pidjezdy/issues/34)) ([cd8918e](https://github.com/Malanius/pidjezdy/commit/cd8918e125de71f48509f1bd966c9198a6051e64))
* **pid:** raise request timeout to 15 seconds ([#72](https://github.com/Malanius/pidjezdy/issues/72)) ([7b25f6f](https://github.com/Malanius/pidjezdy/commit/7b25f6f82b1d97bb14769cf66b3c6efeb21473fc))
* **plugin:** harden settings and error feedback ([#31](https://github.com/Malanius/pidjezdy/issues/31)) ([01966a0](https://github.com/Malanius/pidjezdy/commit/01966a046792cdb68954bb01ce710244539cafa6))
* **plugin:** surface root causes in error details ([#75](https://github.com/Malanius/pidjezdy/issues/75)) ([a53119c](https://github.com/Malanius/pidjezdy/commit/a53119c6135811bf689fb42726e407000d2c5bc8))
* **ui:** avoid overstating cancellation scope ([#82](https://github.com/Malanius/pidjezdy/issues/82)) ([960b33d](https://github.com/Malanius/pidjezdy/commit/960b33dbebebbd5eb6351d5ef290460226c7ef65))
* **ui:** explain all-cancelled departure results ([#74](https://github.com/Malanius/pidjezdy/issues/74)) ([af8bf0a](https://github.com/Malanius/pidjezdy/commit/af8bf0a08e43c1eb1ee46bf0d1345ec4e889530c))
