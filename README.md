# 我说几句，这个项目是基于 https://github.com/TeamFlos/phira-mp 的Github Action自动构建。目前Release的`v1.0.0`为原版（写了绑定ipv6但实际没法连接）和`修复未能成功绑定IPv6`版（详见[TeamFlos/phira-mp-issue15](https://github.com/TeamFlos/phira-mp/issues/15 更改一些代码真正支持了ipv6）
QEMU牛逼！

---

# phira-mp

`phira-mp` is a project developed with Rust. Below are the steps to deploy and run this project.

[简体中文](README.zh-CN.md) | English Version

## Environment

- Rust 1.70 or later

## Server Installation

### For Linux

#### Dependent
First, install Rust if you haven't already. You can do so by following the instructions at https://www.rust-lang.org/tools/install

For Ubuntu or Debian users, use the following command to install `curl` if it isn't installed yet:

```shell
sudo apt install curl
```
For Fedora or CentOS users, use the following command:
```shell
sudo yum install curl
```
After curl is installed, install Rust with the following command:
```shell
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```
Then, build the project:
```shell
cargo build --release -p phira-mp-server
```
#### unning the Server
You can run the application with the following command:
```shell
RUST_LOG=info target/release/phira-mp-server
```

The port can also be specified via parameters:
```shell
RUST_LOG=info target/release/phira-mp-server --port 8080
```

#### Troubleshooting
If you encounter issues related to openssl, ensure that you have libssl-dev (for Ubuntu or Debian) or openssl-devel (for Fedora or CentOS) installed. If the issue persists, you can set the OPENSSL_DIR environment variable for the compilation process.

If you're compiling on Linux and targeting Linux and get a message about pkg-config being missing, you may need to install it:

```shell
# For Ubuntu or Debian
sudo apt install pkg-config libssl-dev 

# For Fedora or CentOS
sudo dnf install pkg-config openssl-devel
```
For other issues, please refer to the specific error messages and adjust your environment accordingly.

#### Monitoring
You can check the running process and the port it's listening on with:
```shell
ps -aux | grep phira-mp-server
netstat -tuln | grep 12345
```
![image](https://github.com/okatu-loli/phira-mp/assets/53247097/b533aee7-03c2-4920-aae9-a0b9e70ed576)

## For Windows or Android
View: [https://docs.qq.com/doc/DU1dlekx3U096REdD](https://docs.qq.com/doc/DU1dlekx3U096REdD)

