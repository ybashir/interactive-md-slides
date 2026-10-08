# Releasing the source

Publish from a fresh source snapshot with one initial commit. Do not reuse the original private repository's `.git`, tags, refs, bundles, or history. Keep private backups and real configuration outside the new repository. A history rewrite does not guarantee that an old hosting provider's cached objects disappear.

The initial public repository will be `ybashir/interactive-md-slides`; the app remains named Interdeck. Leave the existing private repositories unchanged when publishing the new snapshot.

Before preparing the snapshot, run the CI checks, current advisory audits, browser regressions, and a redacted secret scan. Review all tracked source and sample assets, including documentation and binary assets. Confirm MIT ownership and preserve the upstream notices. Update the changelog, configuration examples, and known limits. Commit the reviewed changes.

With Gitleaks 8.30.1 on PATH, run:

```sh
npm run release:prepare
```

The command archives only the committed source, creates a separate repository under the temporary directory with one commit and a GitHub no-reply author address, scans all of its history, and produces a source tarball/checksum. It does not create a remote or publish anything. Review the generated repository before publishing.

For the fresh public GitHub repository, enable private vulnerability reporting, secret scanning/push protection, and Dependabot alerts. Enable branch protection for `main`: require a pull request, successful `verify`, `audit`, `secrets`, container, and CodeQL checks, require resolved conversations, disallow force-pushes and deletion, and apply the protections to administrators. Review the exact check names after the first run. Avoid requiring a second maintainer's approval if no second maintainer exists.

CodeQL deliberately runs only when the repository is public, so its first results must be reviewed before announcing a release. If an action or feature requires a paid GitHub plan for a private staging repository, validate the remaining workflow there and finish public-only checks on the new repository before announcing it.

After the new repository's checks pass, create the version tag/release and announce its link, license, setup instructions, optional provider dependencies, and early-release limitations. Never announce measured event capacity without evidence from the relevant deployment. A self-hosted open-source release does not provision or certify the author's production service.
