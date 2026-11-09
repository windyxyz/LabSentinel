# 更新清单签名公钥（Q-06）

> minisign 密钥对由维护者离线生成（发布决策中的人工事项）。生成后：
> 1. 把**公钥**写入本目录 `update.pub`（纯文本，minisign 公钥格式：
>    `untrusted comment: minisign public key <HEX>` 下一行为 base64 公钥）；
> 2. 私钥离线备份一份，CI 机密存一份（`MINISIGN_SECKEY`）；
> 3. `release.yml` 的 sign 作业解除占位：
>    `minisign -Sm Resources/dist/SHA256SUMS.txt -s "$MINISIGN_SECKEY"`；
> 4. `scripts/release_check.py` 的「产物签名」从 WARN 升级为发布阻断（FAIL）。

**当前状态**：证书/密钥未到位，`update.pub` 不存在；所有发布物保持草稿发布
（`publish.draft: true`），`SHA256SUMS.txt.minisig` 缺失只报 WARN 不阻断。

公钥文件入库是安全的（minisign 公钥本就公开）；**私钥任何情况下都不得入库**，
不得写入源码、示例文件或测试。
