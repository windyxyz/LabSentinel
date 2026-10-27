

## 执行报告

### 交付物
- **服务端**：`GET /api/export/evidence.zip?camera_id=&level=&status=&start_ts=&end_ts=`——
  `export.build_evidence_zip()`（io.BytesIO + zipfile.ZIP_DEFLATED）：events.csv（UTF-8 BOM，列同
  任务 15，抽公共 `event_csv_row()` 复用）+ `snapshots/ev_{id}.jpg` + `videos/ev_{id}.mp4`（存在才打包）
  + `README.txt`（生成时间/筛选条件/数量统计/合规提示）；**上限**：事件 500 条、文件总量 200MB
  （超限 ValueError → API 400 带缩小范围提示）；**路径校验** `_within_data_dir`
  （commonpath 判定，防穿越）；文件名 labsentinel_evidence_<YYYYmmdd_HHMM>.zip
- **Web**：Reports.vue 新增「导出证据包 ZIP」按钮（computed 组 URL：摄像头/日期范围，
  无自定义范围时按 days 折算起止日）
- **文档**：35-api-reference.md 同步端点

### 实跑记录（验收要求）
- TestClient 实跑（隔离 tmp 数据）：`HTTP 200`，ZIP 内容 =
  `['events.csv','README.txt','snapshots/ev_2.jpg','videos/ev_2.mp4','snapshots/ev_1.jpg','videos/ev_1.mp4']`；
  events.csv 首行含中文类型/状态；README 含生成时间/筛选条件/合规提示 ✓（产物 D:	mp\labsentinel_evidence_sample.zip）

### 验收结果
- `pytest tests/ -q`：**78 passed**（新增 4：范围内打包内容与 BOM/事件超限 400/越界路径拒绝入包/API 端点
  与文件名）；`npm run build` 通过（gzip 42.7KB）；api-reference 已同步
- 大文件流式：受 200MB 上限保护，BytesIO 峰值受控（超限前置拒绝）

### 遗留问题
- 200MB 内存峰值在极端场景仍偏高——如需更低内存可换 temp 文件+StreamingResponse（当前上限内可接受）
- ZIP 内不含人脸原图（系统本就未存，符合合规）；README.txt 已含谨慎分发提示
