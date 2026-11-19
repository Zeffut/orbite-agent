// What a contributor and the Maintainer observe of the Contributor Agreement
// on a pull request, with GitHub played by a stand-in that records what it
// is asked.
const test = require('node:test');
const assert = require('node:assert');
const cla = require('./cla.js');

/** A GitHub holding `signatures`, a pull request with `commits` and `comments`. */
function github({ signatures = [], commits = [], comments = [], state = 'open' }) {
  const did = { statuses: [], said: [], kept: null };
  const rest = {
    pulls: {
      get: async () => ({ data: { state, head: { sha: 'abc1234' }, base: { ref: 'main' } } }),
      listCommits: 'commits',
    },
    repos: {
      getContent: async ({ ref, path }) => {
        assert.deepStrictEqual([ref, path], ['cla-signatures', 'signatures.json']);
        return { data: { sha: 'file-sha', content: Buffer.from(JSON.stringify(signatures)).toString('base64') } };
      },
      createOrUpdateFileContents: async (asked) => {
        assert.strictEqual(asked.sha, 'file-sha');
        assert.strictEqual(asked.branch, 'cla-signatures');
        did.kept = JSON.parse(Buffer.from(asked.content, 'base64').toString('utf8'));
      },
      createCommitStatus: async (asked) => did.statuses.push(asked),
    },
    issues: {
      listComments: 'comments',
      createComment: async (asked) => did.said.push(asked.body),
      updateComment: async (asked) => did.said.push(asked.body),
    },
  };
  const paginate = async (what) => (what === 'commits' ? commits : comments);
  return { api: { rest, paginate }, did };
}

const by = (login, sha = 'c0ffee1234') => ({ sha, author: login ? { login } : null });
const context = (payload) => ({ repo: { owner: 'Zeffut', repo: 'orbite-agent' }, payload });
const core = { notice() {} };
const opened = { pull_request: { number: 7 } };
const commenting = (login, body) => ({
  issue: { number: 7, pull_request: {} },
  comment: { body, user: { login, id: 42 }, created_at: '2026-11-20T10:00:00Z' },
});

test('a pull request from someone who has not agreed is held, and told how to agree', async () => {
  const { api, did } = github({ commits: [by('newcomer')] });
  await cla.run({ github: api, context: context(opened), core });
  assert.strictEqual(did.statuses[0].state, 'failure');
  assert.strictEqual(did.statuses[0].sha, 'abc1234');
  assert.match(did.said[0], /@newcomer/);
  assert.ok(did.said[0].includes(cla.PHRASE));
  assert.match(did.said[0], /blob\/main\/CLA\.md/);
});

test('posting the sentence agrees, once and for good', async () => {
  const { api, did } = github({ commits: [by('newcomer')] });
  await cla.run({ github: api, context: context(commenting('newcomer', `  ${cla.PHRASE}\n`)), core });
  assert.deepStrictEqual(did.kept, [
    { login: 'newcomer', id: 42, version: cla.VERSION, pull: 7, at: '2026-11-20T10:00:00Z' },
  ]);
  assert.strictEqual(did.statuses[0].state, 'success');

  // Their next pull request asks nothing and says nothing.
  const later = github({ signatures: did.kept, commits: [by('Newcomer')] });
  await cla.run({ github: later.api, context: context(opened), core });
  assert.strictEqual(later.did.statuses[0].state, 'success');
  assert.deepStrictEqual(later.did.said, []);
  assert.strictEqual(later.did.kept, null);
});

test('the sentence among other words, or changed, does not agree', async () => {
  for (const body of [`Sure. ${cla.PHRASE}`, 'I agree', cla.PHRASE.replace('agree to it', 'do not agree to it')]) {
    const { api, did } = github({ commits: [by('newcomer')] });
    await cla.run({ github: api, context: context(commenting('newcomer', body)), core });
    assert.strictEqual(did.kept, null, body);
    assert.strictEqual(did.statuses[0].state, 'failure', body);
  }
});

test('nobody agrees for somebody else', async () => {
  const { api, did } = github({ commits: [by('newcomer'), by('friend', 'beef5678')] });
  await cla.run({ github: api, context: context(commenting('friend', cla.PHRASE)), core });
  assert.deepStrictEqual(did.kept.map((one) => one.login), ['friend']);
  assert.strictEqual(did.statuses[0].state, 'failure');
  assert.match(did.said[0], /@newcomer/);
  assert.doesNotMatch(did.said[0], /@friend/);
});

test('a commit tied to no account holds the pull request, and is named', async () => {
  const { api, did } = github({ commits: [by(null, 'deadbeef99')] });
  await cla.run({ github: api, context: context(opened), core });
  assert.strictEqual(did.statuses[0].state, 'failure');
  assert.match(did.said[0], /deadbee/);
});

test('the Maintainer and programs are asked nothing', async () => {
  const { api, did } = github({ commits: [by('Zeffut'), by('dependabot[bot]')] });
  await cla.run({ github: api, context: context(opened), core });
  assert.strictEqual(did.statuses[0].state, 'success');
  assert.deepStrictEqual(did.said, []);
});

test('an agreement to an earlier version is asked for again', () => {
  const earlier = [{ login: 'newcomer', id: 42, version: cla.VERSION - 1, pull: 3, at: 'x' }];
  assert.deepStrictEqual(cla.yetToAgree(['newcomer'], earlier), ['newcomer']);
});

test('the message asking to agree is replaced, not repeated, once everyone has', async () => {
  const asked = `<!-- orbite-contributor-agreement -->\nStill to agree: @newcomer`;
  const { api, did } = github({ commits: [by('newcomer')], comments: [{ id: 9, body: asked }] });
  await cla.run({ github: api, context: context(commenting('newcomer', cla.PHRASE)), core });
  assert.strictEqual(did.said.length, 1);
  assert.match(did.said[0], /has agreed/);
});

test('a comment on an issue, or on a closed pull request, is none of this', async () => {
  const onIssue = github({});
  await cla.run({ github: onIssue.api, context: context({ issue: { number: 7 }, comment: { body: cla.PHRASE, user: { login: 'x', id: 1 } } }), core });
  assert.deepStrictEqual(onIssue.did.statuses, []);
  const closed = github({ state: 'closed', commits: [by('newcomer')] });
  await cla.run({ github: closed.api, context: context(commenting('newcomer', cla.PHRASE)), core });
  assert.strictEqual(closed.did.kept, null);
});
