#!/usr/bin/env python3
"""
IcePaw migration checksum 漂移修复工具
======================================
用途：修复启动 panic
    "数据库迁移错误: migration N was previously applied but has been modified"

根因：历史某版安装包的 migration .sql 含未提交改动（dev 工作区污染被打进包），
用户机器 db 记录了那个改动版的 checksum；新版包用 git commit 正版（checksum 不同）
→ sqlx 启动时校验失败 → panic=abort 闪退。

schema 实际一致（同一 ALTER TABLE / CREATE TABLE，仅注释或空白字节差异），
所以同步 checksum 安全，不丢数据。

用法（在闪退的那台机器）：
    1. 确认 IcePaw 已完全退出（任务管理器无 ice-paw.exe）
    2. python fix-checksum-drift.py
       （需 Python 3；Windows 自带或 python.org 装。无 Python 见脚本末尾备选。）
"""
import sqlite3, shutil, os, sys

DB = os.path.join(os.environ.get("APPDATA", "."), "com.icepaw.app", "ice-paw.db")
if not os.path.exists(DB):
    print(f"[X] 找不到 db: {DB}")
    print("    确认 %APPDATA%\\com.icepaw.app\\ice-paw.db 存在，且 IcePaw 在本机装过至少一次。")
    sys.exit(1)

# 0.3.0 git commit 版 migration checksums（version -> sha384 hex，小写）
# 来源：packages/app/src-tauri/src/db/migrations/*.sql 的 SHA-384
CORRECT = {
    1:  "47994440262dfd4107e26302963a2e4d1c74c9551632a9dfe9cfe612db6eb76bf191ecd9e474529e3f066768f6dea7ab",
    2:  "1914eada9caade97f2d63356abec74c484e11e1f31d8198a4d1916acb9ad83c4f996f4b2a6cbc19011d9c5011911a209",
    3:  "599cac8e3fae44659a5faa874783e38cbf1b5390c12fd7dd9316f90f4a3412e2dadd59f62413059d453eb702e95319a5",
    4:  "329c244f20675d7ec6eeba2be37ef06c6591521346594f4fa6bbc6aedba5816f310ba98ebd99b324651943e544cab005",
    5:  "2f28bec04df8187d081536d62968dce7df0d42dcd284e12ac4cf4f3b4f89d91c027efe51ca87be4201c56ebf58834a71",
    6:  "dea457d0202ac5ce8e35278045c5196e0f668ab4d0a8c8f01aff79e40fd4ba9da312425a8802fecd59b6139c65132a31",
    7:  "9dea85336b19941adeafc6cc1cf64b74c028c19125f69f2cf705240971662acfde162b132de742900e992b43b8142268",
    8:  "f0144e9ba24c2d22b179d7adc5a77a0b32b5590953c6c5944ce3515ca38a15c80e54e8f8f4ca727649272e24f04c8bde",
    9:  "bf0a5d7f5649e5c2486efe83a8bb991da77a5485cbc3a30c19d87194739be8e333dea58b23b0f4fb5b4ea9c47e9cde56",
    11: "bad53f9fe7a599f7446c151efb0d961f5e8a7d1c7b23c00975716924ddd828f7b8faab87ea4bed49d6f136cfb17ad307",
    12: "655246fbfc5f6c462d9a19e7d8151086622ffa97ed31817314bbfaf824a6c9622b08441da87a7be976d69daad5582d26",
    13: "4223dcdaa5bcc181484eb3d56e266cb90552e3a45c331b77278f4f8283957986ee07a22f28a5717d49a00f21f7f2da95",
    14: "594a01cca7682d135ac0a8b8a5a3c90a154473df75ff58b11d39c70b9a623d41d5290b6106f173a9861fd07bb7dfcd1e",
    15: "f8604d32db14084bbd94b5073c5b45a53dc787d650e5a8b901521940a5f57e7aae11d094dcd91f6995c80c7b85005992",
    16: "79d71de115955690c182c3dd6a59b7bfce389ff27d6b43f275f0c6c4ab6358def833d1a703c8154ac0b5cb9ae9f3cc08",
    17: "314510e4607d3cca1f26a942b916c348f2e8b9b067c83b0dcb559642ff681b55d2b0bf30916ceaf91764614d07bf41dc",
    18: "78d5352e6ec5bb52b27d8a8f4e7371e14094c718ea8aa027a10401a2c4db357af3f3b8b1f3bd0bb34233056f9fbc0bb9",
    19: "88bcbcc8f558ce8628aa93afa23fdb9f340948c04430890a8180f01b8963286814214cf07b8e5f4ce6e08025c81b2726",
    20: "57f18785bd00bd473010e7c775ee49cc3513e8a8868012263c762434d02c59e748cd08a9f717070a9624eda4e9fa53f5",
    21: "5a44d8ff424d448c27f4c10095ed336c4e53efb1231307ced7240e5c46a1f6f0e4f9b10f46ce7a7462cb80d17d3d38b5",
    22: "f6bbf8504270157034ffbf1e60d3966bd376a8bc9a14269dea11b0618d967a214a2b0780ac2165c632c2a8a1c26a4472",
    23: "29c3ac0ea424a9469ccac6437ac3cfd28f2e7e8f96007792963040cfdf71d6d1ff30c9b4c51ffe0326c6acdb8a3ffe94",
    24: "7e7f0989963782b6d033a16489af48fe047ad68c7ad010dc21dbbbb1208acff2baaaa4cead3be0d039c6dd8686ed67dd",
    25: "da8df9601827c202c6c021621417f7a2c5f381be7f8848675332c61d8710c24b0cd4adba9f3c86d5829ceb4863082394",
    26: "f49c966f228eacaca10d7e5a8fd9acfe8a6161d97393f620915c7901d12437c33d42ed665a42ec8a30e44709fd2318e8",
    27: "fe55f4ce69f60c3b7a1c7f19e23d2311851ccb8106f16e3e0968f8317d565ce39079467dfc91a6ec3e106db502a42f30",
    28: "db1fc42cfa6b173605f985a13a3545bb9a1cf3d5fe89564ee99703bc887110048bdf789dc214fb93255ba324214ba954",
    29: "81fe064ef31ee85cb386c0f55a69e3d6ecf05394c7e52a086635e6fa8a4857ac6ddf808512ac1f71be252e9de116ad00",
    30: "26a2b7559fb60f867c1cd9b11a7448bb8a4db370a37cfbce2c0faccb548e22122e2db18c9301a6902248f570e842624f",
    31: "2e9dccfe7d980cf3dbbf6d1617572ed6b9372c0d90f478dc9c6e123fafd5e9474646ede7d73d1e40b3c8228f898d71eb",
    32: "fb42894f0a1d37e3500f39bdcf8e211e6ddbf0cb0431d7e03c23c36de75f5cc8d4dec3b8e8c88f9aed82e8077e39e0cb",
    33: "72711a2251cdf9115586db37622631cb9d4f6098cbd40928c1c0d43b8b1637062cc22b90f2488ae8236d25b039b4cf4b",
    34: "ec8ece67b6e6815c4f233c8e8caef5090006a85e9cd8571ddb36ddec40d10d7bb649d123f4fbd2ae78371d4f1a19230c",
    35: "d19654a5fe4723160bb6e216d884fe97870e4b709e3137debbc1dcab6b68437bec53a32ac959f428ed13f109b3a29d01",
    36: "3d0b7db38c60cbfc13351d9e16e32f9c541d2a16653025412b2418ba4e6a6008266e94e3579f7d7c21d347be61432a08",
    37: "0ada70cd728e60bd7070d977bf0b11dc3640df453b3db8b2af887bc238f65eae8e943abb889c75621c5625e7ac8d24a0",
    38: "4feb42b744bf0d0d0af33923fd5d546149eead30086ee29f40a8c615e0e0555ad7f9f9c295cbb95f274ef9a472e69554",
    39: "a4fa440c9b31284eb8c620e7c88a1a0e5e79ad8bfc13a774e368914feb2b3252748e94acacad70c263ed8f425d2adcd0",
    40: "4046f71c867de9d8a1c9adee6a474fab9f708aa861d8b5855b2c4c4b6f7db912c162f7a5339547376f35eb3675db5115",
    41: "626831600b16c0c40440b90bea0feadb7cfde4bf249a81556998b4833bdd60052944c1400848174ec10de9335196cd73",
    42: "e11965076ddd97a66d28505b50aff7575396ff17a25304a2c17bddecead21cd4f21a677f6fcf35b5490905033f8e567d",
    43: "7d44f8950729a553017d2f8414e80327b56bda6f71542afe20f1232a6df2792d4bdf9e60f31de87a418e6a44b6cb2074",
}

# 1) 备份
bak = DB + ".checksum-fix.bak"
shutil.copy2(DB, bak)
print(f"[1/3] 已备份 db -> {bak}")

# 2) 对比 + UPDATE 不匹配的
c = sqlite3.connect(DB)
rows = list(c.execute("SELECT version, hex(checksum) FROM _sqlx_migrations ORDER BY version"))
fixed = []
unknown = []  # db 里有但 CORRECT 没有（新 migration？）
for v, h in rows:
    cur = (h or "").lower()
    want = CORRECT.get(v)
    if want is None:
        unknown.append(v)
    elif want != cur:
        c.execute("UPDATE _sqlx_migrations SET checksum=? WHERE version=?", (bytes.fromhex(want), v))
        fixed.append((v, cur, want))

# 3) schema 兼容性快查（migration 24 = agents.workspace_path）
cols = [r[1] for r in c.execute("PRAGMA table_info(agents)")]
c.commit()

print(f"[2/3] 校验 {len(rows)} 条 migration 记录，修复 {len(fixed)} 个 checksum:")
for v, old, want in fixed:
    print(f"    v{v}: {old[:16]}… -> {want[:16]}…")
if not fixed:
    print("    (无漂移——若仍 panic，把本脚本完整输出发给开发)")
if unknown:
    print(f"    [注意] db 有但脚本未覆盖的 version: {unknown}（可能是更新的 migration，正常）")

print("[3/3] schema 兼容性:")
if "workspace_path" in cols:
    print("    OK: agents.workspace_path 已存在（migration 24 schema 已生效）")
else:
    print("    [X] agents 表缺 workspace_path 列：migration 24 的 schema 未真正生效。")
    print("        光改 checksum 不够，需补跑: ALTER TABLE agents ADD COLUMN workspace_path TEXT DEFAULT NULL;")
    c.execute("ALTER TABLE agents ADD COLUMN workspace_path TEXT DEFAULT NULL")
    c.commit()
    print("    已自动补加该列。")

print("\n完成。现在启动 IcePaw 应正常。若仍 panic，把新的 panic 信息发给开发。")
print(f"如需回滚：关闭 IcePaw，用备份 {bak} 覆盖 db。")
