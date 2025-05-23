FROM ghcr.io/cucapra/calyx:latest

LABEL org.opencontainers.image.source https://github.com/cucapra/filament

# Install apt packages
RUN apt-get update -y && \
  apt-get -y install unzip

# Install CVC5
WORKDIR /home
ENV PATH=$PATH:/root/.local/bin
RUN wget https://github.com/cvc5/cvc5/releases/download/cvc5-1.0.6/cvc5-Linux --output-document cvc5 && \
  chmod +x cvc5 && \
  mv cvc5 /root/.local/bin

# Install z3
WORKDIR /home
RUN mkdir z3
WORKDIR /home/z3
RUN wget https://github.com/Z3Prover/z3/releases/download/z3-4.12.2/z3-4.12.2-x64-glibc-2.31.zip --output-document z3.zip && \
  unzip z3.zip
ENV PATH=$PATH:/home/z3/z3-4.12.2-x64-glibc-2.31/bin/

# ----------------------------------------
# Install custom tools
# ----------------------------------------
WORKDIR /home

# Install required libraries
RUN apt update && \
    DEBIAN_FRONTEND=noninteractive apt install -y autoconf automake \
    autotools-dev bison f2c flex git gpg g++ libblas-dev libboost-all-dev \
    liblapack-dev liblpsolve55-dev libsollya-dev libtool lp-solve ninja-build \
    pkg-config sollya wget


# Install latest cmake from source
RUN wget https://github.com/Kitware/CMake/releases/download/v3.28.0-rc3/cmake-3.28.0-rc3.tar.gz && \
    tar -xvf cmake-3.28.0-rc3.tar.gz && \
    cd cmake-3.28.0-rc3 && ./bootstrap &&\
    make && make install

# Install FloPoCo 4.1
WORKDIR /home
RUN git clone https://gitlab.com/flopoco/flopoco &&\
    cd flopoco && git checkout 418ee8b71fe2c4976cbc0a92924683fa73162128 &&\
    make sysdeps CONFIG=docker &&\
    make CONFIG=docker &&\
    make install CONFIG=docker

# Install GHDL 3.0.0
WORKDIR /home
# Install deps
RUN apt install -y gnat llvm clang
RUN git clone --depth 1 --branch v3.0.0 https://github.com/ghdl/ghdl.git &&\
    cd ghdl &&\
    mkdir build && cd build &&\
    LDFLAGS='-ldl' ../configure --with-llvm-config --prefix=/usr &&\
    make && make install

# Install verible
WORKDIR /home
RUN wget https://github.com/chipsalliance/verible/releases/download/v0.0-3428-gcfcbb82b/verible-v0.0-3428-gcfcbb82b-Ubuntu-20.04-focal-x86_64.tar.gz --output-document verible.tar.gz && \
  tar -xvf verible.tar.gz && \
  mv verible-v0.0-3428-gcfcbb82b verible && \
  rm verible.tar.gz
ENV PATH=$PATH:/home/verible/bin

# Set rust to 1.82 and runt to 0.4.1
RUN rustup toolchain install 1.85.0 &&\
    rustup default 1.85.0 &&\
    cargo install runt --version 0.4.1

# ----------------------------------------
# Install filament
# ----------------------------------------
WORKDIR /home
ADD . filament
# Build the compiler
WORKDIR /home/filament
# Make rust use sparse registries (https://doc.rust-lang.org/nightly/cargo/reference/registries.html#registry-protocols)
ENV CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
# Set up fud
RUN python3 -m pip install cocotb find_libpython pytest && \
  fud register -p fud/filament.py filament && \
  fud config stages.filament.exec /home/filament/target/debug/filament && \
  fud config stages.filament.library /home/filament