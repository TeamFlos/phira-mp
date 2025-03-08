FROM ubuntu:22.04

RUN apt-get update && apt-get -y upgrade && apt-get install -y curl git build-essential pkg-config openssl libssl-dev

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"
WORKDIR /root/
RUN git clone -b fix--WinIPv6 https://github.com/afoim/phira-mp-autobuild.git
WORKDIR /root/phira-mp-autobuild
RUN cargo build --release -p phira-mp-server

ENTRYPOINT ["/root/phira-mp-autobuild/target/release/phira-mp-server", "--port", "<preferred-port>"]
