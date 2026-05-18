# PingInfo

PingInfo 是一个轻量的 Web 监控工具。

它支持：

- ICMP 探测
- TCP 端口探测
- 内置 Web UI
- SQLite 本地存储
- 最近记录、实时图表、失败定位
- Docker 部署

## 默认配置

本地默认值：

- 监听地址：`0.0.0.0:18080`
- 数据库：`data/pinginfo.db`
- 保留天数：`30`

可用环境变量覆盖：

- `PINGINFO_BIND`
- `PINGINFO_DB`
- `PINGINFO_RETENTION_DAYS`

也可以用启动参数覆盖监听地址：

- `-h` / `--host`
- `-p` / `--port`

## 本地运行

进入项目目录：

```bash
cd /path/to/pinginfo
```

首次编译：

```bash
cargo build
```

启动：

```bash
mkdir -p data
./target/debug/pinginfo
```

指定地址或端口：

```bash
./target/debug/pinginfo -h 127.0.0.1 -p 18080
```

打开：

```text
http://localhost:18080
```

## 直接运行

也可以直接从 GitHub Releases 下载对应平台压缩包。  
解压后进入目录，直接运行：

```bash
./pinginfo
```

Windows 也可以直接运行：

```powershell
.\pinginfo.exe -h 127.0.0.1 -p 18080
```

查看版本：

```bash
./pinginfo --version
```

程序目录里会自带 `static/`，Web UI 可以直接使用。

## ICMP 权限

如果要让 ICMP 正常工作，需要给二进制加 `cap_net_raw`。

```bash
sudo setcap cap_net_raw+ep /usr/local/bin/pinginfo
```

## Docker

构建镜像：

```bash
docker build -t pinginfo .
```

也可以直接使用 Compose：

```bash
docker compose up -d
```

推荐使用 named volume：

```bash
docker volume create pinginfo-data
docker run -d \
  --name pinginfo \
  --cap-add=NET_RAW \
  -p 8080:8080 \
  -v pinginfo-data:/app/data \
  charley008/pinginfo:latest
```

打开：

```text
http://localhost:8080
```


## 目录

```text
pinginfo/
  src/         Rust 后端
  static/      Web UI
  data/        本地数据库（运行时生成）
  target/      编译产物（运行时生成）
```

## 当前功能

- 多目标并发监控
- 目标增删改
- 批量导入地址
- 最近记录最多显示 1000 条
- 图表支持 10 分钟到 24 小时
- 24 小时异常事件摘要
- 清空当前目标数据
