# Maintainer: Ricarsz <ricarsz@users.noreply.github.com>
pkgname=niri-dock
pkgver=0.3.8
pkgrel=1
pkgdesc="macOS-style dock for the niri Wayland compositor"
arch=('x86_64')
url="https://github.com/Ricarsz/niri-dock"
license=('MIT')
depends=('gtk4' 'gtk4-layer-shell')
makedepends=('cargo' 'rust')
options=('!debug' 'strip')
source=("$pkgname-$pkgver.tar.gz::$url/archive/v$pkgver.tar.gz")
sha256sums=('SKIP')

prepare() {
    cd "$pkgname-$pkgver"
    export CARGO_HOME="$srcdir/cargo-home"
    # 不用 --locked：万一 tag 里的 Cargo.lock 与 Cargo.toml 版本号不同步，
    # cargo 会直接拒绝构建（0.3.1 的 release 就是这么挂的）
    cargo fetch
}

build() {
    cd "$pkgname-$pkgver"
    export CARGO_HOME="$srcdir/cargo-home"
    # 不把构建机的家目录写进二进制（panic 路径等）
    export RUSTFLAGS="--remap-path-prefix=$srcdir=/build --remap-path-prefix=${HOME:-/root}=/build"
    cargo build --release --offline
}

package() {
    cd "$pkgname-$pkgver"
    install -Dm755 "target/release/$pkgname" "$pkgdir/usr/bin/$pkgname"
    install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
