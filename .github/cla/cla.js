// Who has agreed to the Orbite Contributor Agreement (CLA.md), and whether
// everyone whose commits are in a pull request has.
//
// An agreement is the sentence below, posted alone as a comment on a pull
// request. It is kept, with who posted it and when, in signatures.json on
// the branch `cla-signatures`, which holds nothing else.

const PHRASE = 'I have read the Orbite Contributor Agreement and I agree to it.';
// Raised when CLA.md changes in a way people must agree to again.
const VERSION = 1;
const BRANCH = 'cla-signatures';
const FILE = 'signatures.json';
const MARK = '<!-- orbite-contributor-agreement -->';
// Those who need not agree: the Maintainer, and programs.
const EXEMPT = ['Zeffut', 'dependabot[bot]', 'github-actions[bot]'];

/** True when `body` is the sentence that agrees, and nothing else. */
function agrees(body) {
  return typeof body === 'string' && body.trim() === PHRASE;
}

/** The accounts behind `commits`, and the commits no account stands behind. */
function authorsOf(commits) {
  const logins = new Set();
  const unknown = [];
  for (const commit of commits) {
    const login = commit.author && commit.author.login;
    if (login) logins.add(login);
    else unknown.push(commit.sha.slice(0, 7));
  }
  return { logins: [...logins].sort(), unknown };
}

/** Of `logins`, those who have not agreed to this version. */
function yetToAgree(logins, signatures) {
  const agreed = new Set(
    signatures.filter((one) => one.version === VERSION).map((one) => one.login.toLowerCase()),
  );
  const exempt = new Set(EXEMPT.map((login) => login.toLowerCase()));
  return logins.filter((login) => !agreed.has(login.toLowerCase()) && !exempt.has(login.toLowerCase()));
}

/** `signatures` with the agreement of `user`, given in `pull` at `at`, unless it is there. */
function withAgreement(signatures, user, pull, at) {
  const there = signatures.some(
    (one) => one.version === VERSION && one.login.toLowerCase() === user.login.toLowerCase(),
  );
  if (there) return signatures;
  return [...signatures, { login: user.login, id: user.id, version: VERSION, pull, at }];
}

/** What the pull request is told. */
function said(missing, unknown, agreementUrl) {
  if (missing.length === 0 && unknown.length === 0) {
    return `${MARK}\nEveryone whose work is in this pull request has agreed to the [Orbite Contributor Agreement](${agreementUrl}). Thank you.`;
  }
  const lines = [MARK, `Thank you for contributing. Before this can be merged, the [Orbite Contributor Agreement](${agreementUrl}) has to be agreed to by everyone whose commits are in it. It is short, and you do it once.`];
  if (missing.length > 0) {
    lines.push('', `Still to agree: ${missing.map((login) => `@${login}`).join(', ')}. To agree, post this sentence, alone, as a comment here:`, '', `> ${PHRASE}`);
  }
  if (unknown.length > 0) {
    lines.push('', `These commits are not tied to a GitHub account, so nobody can agree for them: ${unknown.join(', ')}. Add the address they were made with to your account, or make them again with one that is.`);
  }
  return lines.join('\n');
}

async function run({ github, context, core }) {
  const { owner, repo } = context.repo;
  const number = context.payload.pull_request
    ? context.payload.pull_request.number
    : context.payload.issue && context.payload.issue.pull_request && context.payload.issue.number;
  if (!number) return;
  const { data: pull } = await github.rest.pulls.get({ owner, repo, pull_number: number });
  if (pull.state !== 'open') return;

  const kept = await github.rest.repos.getContent({ owner, repo, path: FILE, ref: BRANCH });
  let signatures = JSON.parse(Buffer.from(kept.data.content, 'base64').toString('utf8'));

  const comment = context.payload.comment;
  if (comment && agrees(comment.body)) {
    const next = withAgreement(signatures, comment.user, number, comment.created_at);
    if (next !== signatures) {
      await github.rest.repos.createOrUpdateFileContents({
        owner, repo, path: FILE, branch: BRANCH, sha: kept.data.sha,
        message: `${comment.user.login} agreed to the Contributor Agreement in #${number}`,
        content: Buffer.from(`${JSON.stringify(next, null, 2)}\n`).toString('base64'),
      });
      signatures = next;
    }
  }

  const commits = await github.paginate(github.rest.pulls.listCommits, { owner, repo, pull_number: number, per_page: 100 });
  const { logins, unknown } = authorsOf(commits);
  const missing = yetToAgree(logins, signatures);
  const agreed = missing.length === 0 && unknown.length === 0;

  await github.rest.repos.createCommitStatus({
    owner, repo, sha: pull.head.sha, context: 'Contributor agreement',
    state: agreed ? 'success' : 'failure',
    description: agreed ? 'Agreed to by everyone in this pull request' : 'Not yet agreed to by everyone in this pull request',
  });

  const agreementUrl = `https://github.com/${owner}/${repo}/blob/${pull.base.ref}/CLA.md`;
  const text = said(missing, unknown, agreementUrl);
  const comments = await github.paginate(github.rest.issues.listComments, { owner, repo, issue_number: number, per_page: 100 });
  const mine = comments.find((one) => one.body && one.body.startsWith(MARK));
  if (mine && mine.body !== text) {
    await github.rest.issues.updateComment({ owner, repo, comment_id: mine.id, body: text });
  } else if (!mine && !agreed) {
    // Nothing is said on a pull request where there is nothing to agree to.
    await github.rest.issues.createComment({ owner, repo, issue_number: number, body: text });
  }
  if (!agreed) core.notice('The Contributor Agreement is not yet agreed to by everyone in this pull request.');
}

module.exports = { PHRASE, VERSION, agrees, authorsOf, yetToAgree, withAgreement, said, run };
