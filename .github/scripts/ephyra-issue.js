// Opens an issue when the scheduled ephyra search fails, or comments on the
// open one. Called from .github/workflows/ephyra.yml through
// actions/github-script, with the run's report in `reportPath`.
const fs = require('fs');

const MARKER = '<!-- ephyra-search-failure -->';

// Seeds are u64 values beyond 2^53, which JSON.parse would round. They are
// quoted first, so replay lines name the stored input exactly.
function readReports(reportPath) {
  if (!fs.existsSync(reportPath)) {
    return [];
  }
  return fs
    .readFileSync(reportPath, 'utf8')
    .split('\n')
    .filter((line) => line.trim() !== '')
    .map((line) => JSON.parse(line.replace(/"seed":(\d+)/g, '"seed":"$1"')));
}

// One Markdown entry per distinct failure of every failed property.
function describeFailures(reports) {
  const entries = [];
  for (const report of reports) {
    const found = report.failures?.length
      ? report.failures
      : report.failure
        ? [{ failure: report.failure, count: 1 }]
        : [];
    for (const { failure, count } of found) {
      const where = failure.location
        ? `${failure.location.file}:${failure.location.line}`
        : 'unknown location';
      const message = (failure.message || '').split('\n')[0].slice(0, 300).replaceAll('`', "'");
      const replay = failure.replay
        ? `EPHYRA_REPLAY=${failure.replay.seed}:${failure.replay.iteration} cargo test --release ${report.name}`
        : 'no replay point recorded';
      entries.push(
        `- \`${report.name}\` at \`${where}\`, seen ${count} times${failure.flaky ? ' (flaky)' : ''}\n` +
          `  \`${message}\`\n` +
          `  \`${replay}\``,
      );
    }
  }
  return entries;
}

module.exports = async ({ github, context, reportPath }) => {
  const runUrl = `${context.serverUrl}/${context.repo.owner}/${context.repo.repo}/actions/runs/${context.runId}`;
  const entries = describeFailures(readReports(reportPath));
  const failures = entries.length
    ? entries.join('\n')
    : 'The run failed before ephyra wrote a report; see the run log.';
  const body = [
    MARKER,
    `The [property test search](${runUrl}) found a counterexample.`,
    '',
    failures,
    '',
    'Run the replay lines in `ephyra-tests/`. The new regression entries are in the run\'s',
    '`ephyra-counterexamples` artifact; commit them together with the fix.',
  ].join('\n');

  const { owner, repo } = context.repo;
  const open = await github.paginate(github.rest.issues.listForRepo, {
    owner,
    repo,
    state: 'open',
    per_page: 100,
  });
  const existing = open.find((issue) => !issue.pull_request && issue.body?.includes(MARKER));
  if (existing) {
    await github.rest.issues.createComment({ owner, repo, issue_number: existing.number, body });
  } else {
    await github.rest.issues.create({
      owner,
      repo,
      title: 'Property tests found a counterexample',
      body,
    });
  }
};

module.exports.describeFailures = describeFailures;
module.exports.readReports = readReports;
