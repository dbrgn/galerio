# Releasing

Set variables:

    $ export VERSION=X.Y.Z

Update version numbers:

    $ vim Cargo.toml
    $ cargo update -p galerio

Update changelog:

    $ vim CHANGELOG.md

Commit & tag:

    $ git commit -m "Release v${VERSION}"
    $ git tag -a v${VERSION} -m "Version ${VERSION}"

Publish:

    $ git push && git push --tags
