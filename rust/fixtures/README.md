# 测试样本说明

全部为合成输入，除SQLite结构与API模型外不含生产行数据。不能用于替换生产config或模拟“真实任务已跑完”。

- `acceptance-cases.json`：完整用例矩阵，每个ID必须映射一个或多个可执行测试。
- `golden-naming.json`：独立预期，不能由被测函数生成expected。
- `auth.synthetic.json`：独立Node crypto生成的旧PBKDF2/HMAC样本；时钟须冻结到frozenNowUnix。
- `legacy-task-fixtures.json`：schema1/8个workflow检查点的合成旧任务；目录/URL均为测试输入，恢复测试必须配套stub真实文件状态。
- `config.synthetic.toml`：secret marker、未知section与注释保持用例；所有端口替换为测试harness实际监听端口。
- RSS文件：历史问题的合成feed；enclosure长度仅示例值，stub使用真实响应Content-Length。
- TMDB文件：错候选/钉选ID的合成资料，日期/海报不是公网事实，也不能成为生产硬编码特殊case。
- `torrent-single-v1.torrent`：空payload的parser样本，可核验infohash，不能用它证明helper不下载pieces。
- `torrent-metadata-seed-v1.torrent` / `.payload`：128KiB非零合成数据，原生helper须连本地seed并证明无piece请求/数据下载。

开发者可扩展fixture，不能通过修改本包expected来掩盖实现错误。修改明确要求的行为须说明理由、更新版本并取得相应确认。
现有病例默认是最小回归集，不代替每个外部接口的完整故障/边界数据。
