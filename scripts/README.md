# scripts/ · 启动/停止/健康检查脚本（任务18）

| 脚本 | 作用 | 说明 |
|---|---|---|
| `start-all.bat` | 一键启动 go2rtc(1984) + 服务端(8000) + vite(5173) | 端口被占用则**跳过**该组件（不报错退出）；环境预检（venv/node_modules 缺失给安装命令）；日志落 `logs/<name>.log`（目录自动创建） |
| `stop-all.bat` | 一键停止 | 按端口找 PID `taskkill /F`，带确认；顺带关闭名为 go2rtc/server/web 的控制台窗口壳 |
| `health-check.bat` | 状态表 | curl 探测 API/system、go2rtc /api、Web（vite 仅绑 IPv6，用 `[::1]` 探测） |

## 用法

```bat
scripts\start-all.bat     :: 启动（可重复执行，已占用端口自动跳过）
scripts\health-check.bat  :: 状态表 [OK]/[DOWN]
scripts\stop-all.bat      :: 停止（Y/N 确认）
```

## 已知坑

1. **编码**：三个 .bat 必须是 **GBK/ANSI 编码**（cmd 按 OEM 代码页解析批处理；UTF-8 保存会把中文
   echo 变成乱码甚至破坏语法）。用编辑器修改后请转回 ANSI 再保存。
2. vite dev 仅绑 IPv6 localhost（README 已知事项），health-check 用 `[::1]:5173` 探测。
3. `stop-all.bat` 会停掉**所有**占用 1984/8000/5173 的进程——若有其他服务误用这三个端口请先挪开。
4. 不自动启动训练/推理（训练由巡检按《训练安全规程》管理）。
