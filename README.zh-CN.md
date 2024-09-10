# phira-mp

`phira-mp` 是一个用 Rust 开发的项目。 以下是部署和运行该项目服务端的步骤。

简体中文 | [English Version](README.md)
## 环境

- Rust 1.70 或更高版本

## 服务端安装
### 对于 `Linux` 用户
#### 依赖
首先，如果尚未安装 Rust，请安装。 您可以按照 https://www.rust-lang.org/tools/install 中的说明进行操作

对于 Ubuntu 或 Debian 用户，如果尚未安装“curl”，请使用以下命令进行安装：

```shell
sudo apt install curl
```
对于 Fedora 或 CentOS 用户，请使用以下命令：
```shell
sudo yum install curl
```
安装curl后，使用以下命令安装Rust：
```shell
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```
然后，构建项目：
```shell
cargo build --release -p phira-mp-server
```
#### 运行服务端
您可以使用以下命令运行该应用程序：
```shell
RUST_LOG=info target/release/phira-mp-server
```

也可以通过参数指定端口：
```shell
RUST_LOG=info target/release/phira-mp-server --port 8080
```

### For docker

1. 创建 Dockerfile
```
FROM ubuntu:22.04

RUN apt-get update && apt-get -y upgrade && apt-get install -y curl git build-essential pkg-config openssl libssl-dev

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"
WORKDIR /root/
RUN git clone https://github.com/TeamFlos/phira-mp
WORKDIR /root/phira-mp
RUN cargo build --release -p phira-mp-server

ENTRYPOINT ["/root/phira-mp/target/release/phira-mp-server", "--port", "<preferred-port>"]
```

2. 构建镜像
`docker build --tag phira-mp .`

3. 运行容器
`docker run -it --name phira-mp -p <prefered-port>:<preferred-port> --restart=unless-stopped phira-mp`

#### 故障排除
如果遇到与 openssl 相关的问题，请确保安装了 libssl-dev（适用于 Ubuntu 或 Debian）或 openssl-devel（适用于 Fedora 或 CentOS）。 如果问题仍然存在，您可以为编译过程设置 OPENSSL_DIR 环境变量。

如果您在 Linux 上进行编译并以 Linux 为目标，并收到有关缺少 pkg-config 的消息，则可能需要安装它：

```shell
# 对于 Ubuntu 或 Debian
sudo apt install pkg-config libssl-dev 

# 对于 Fedora 或 CentOS
sudo dnf install pkg-config openssl-devel
```
其他问题请参考具体错误信息并相应调整您的环境。

#### 监控
您可以检查正在运行的进程及其正在侦听的端口：
```shell
ps -aux | grep phira-mp-server
netstat -tuln | grep 12345
```
![image](https://github.com/okatu-loli/phira-mp/assets/53247097/b533aee7-03c2-4920-aae9-a0b9e70ed576)

## 对于 Windows 或 Android 用户
查看: [https://docs.qq.com/doc/DU1dlekx3U096REdD](https://docs.qq.com/doc/DU1dlekx3U096REdD)
