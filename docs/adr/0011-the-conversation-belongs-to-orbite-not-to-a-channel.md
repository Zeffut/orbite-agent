# The Conversation belongs to Orbite, not to a Channel

Telegram is the only Channel in 1.0, but a web interface, a mobile app and a desktop app are to follow, and the Owner must find the same Conversation in all of them: not a Telegram chat here and a web chat there. The Conversation is therefore kept by Orbite itself, in its own store and in a form no Channel owns: what the Owner said, what the Agent said and asked to do (in Markdown, as the model wrote it), what came of each Action, the notices, and the files as paths on the Host. A Channel only shows it and brings back what the Owner does: it translates at the edge (Markdown to Telegram's formatting, an Approval to buttons, the steps of the work to a status message) and stores nothing of its own that the others would need.

## Consequences

- Nothing specific to a Channel may enter the stored Conversation: no Telegram message identifiers, no formatting dialect, no Channel-only state.
- Everything the Owner can be shown or asked (answers, files, Approvals, steps, notices) goes through the one Channel interface, so that a new Channel is a new implementation of it and nothing else.
- Several Channels at once on the one Conversation, with an Approval asked in one answerable from another, is the next step and needs the pending Approval to live in Orbite rather than in the Channel that shows it.
