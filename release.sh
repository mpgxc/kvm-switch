#!/bin/sh
# Publica uma versão nova: ./release.sh 0.2.0
# Põe a versão no Cargo.toml e no Cargo.lock, commita, cria a tag v<versão> e envia
# a main e a tag juntas. O push da tag dispara .github/workflows/release.yml, que
# gera o .dmg e o .deb e cria a release no GitHub.
# Sem argumento, publica a versão que já está no Cargo.toml.
set -eu
cd "$(dirname "$0")"

current=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
version=${1:-$current}
tag="v$version"

echo "$version" | grep -Eqx '[0-9]+\.[0-9]+\.[0-9]+' ||
  { echo "versão inválida: $version (use X.Y.Z)" >&2; exit 2; }
[ "$(git branch --show-current)" = main ] || { echo "rode na branch main" >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "há mudanças não commitadas" >&2; exit 1; }
git fetch --quiet --tags origin
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] ||
  { echo "a main local difere da origin/main: faça pull ou push antes" >&2; exit 1; }
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  echo "a tag $tag já existe" >&2
  exit 1
fi

if [ "$version" != "$current" ]; then
  # Só a primeira linha "version =" é a do pacote; as outras são de dependências.
  sed "1,/^version = /s/^version = .*/version = \"$version\"/" Cargo.toml >Cargo.toml.novo
  mv Cargo.toml.novo Cargo.toml
  # Atualiza só a versão do próprio pacote no Cargo.lock, sem mexer nas dependências.
  cargo update --workspace --quiet
  git commit --quiet -m "release $tag" Cargo.toml Cargo.lock
fi

git tag -a "$tag" -m "$tag"
git push --atomic origin main "$tag"

echo "tag $tag enviada; a release sai em https://github.com/mpgxc/kvm-switch/actions"
