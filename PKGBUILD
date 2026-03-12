# Maintainer: Radek Podgorny <radek@podgorny.cz>
pkgname=agentcat-git
provides=('agentcat')
conflicts=('agentcat')
pkgver=r5.408fdc3
pkgrel=1
pkgdesc="Universal stream formatter for AI coding agent output"
arch=('x86_64')
url="https://github.com/rpodgorny/agentcat"
license=('GPL-3.0-or-later')
makedepends=('git' 'cargo')
source=("$pkgname::git+https://github.com/rpodgorny/agentcat")
md5sums=('SKIP')

pkgver() {
	cd "$srcdir/$pkgname"
	printf "r%s.%s" "$(git rev-list --count HEAD)" "$(git rev-parse --short HEAD)"
}

build() {
	cd "$srcdir/$pkgname"
	cargo build --release --locked
}

package() {
	cd "$srcdir/$pkgname"
	install -D -m 0755 -t $pkgdir/usr/bin/ target/release/agentcat
}
