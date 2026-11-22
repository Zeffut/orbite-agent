# The Agent's Computer is the machine's own desktop, or one of its own

The Agent's Computer comes in two kinds, on the Host:

- **This computer**: the desktop people see on the machine's screen, with its programs, its windows and the accounts signed in there. What the Agent does, its Owner watches, and can take over with the keyboard.
- **A computer of its own**: a screen nobody looks at, with a browser that is the Agent's alone. Nothing of the Owner's is on it.

**This computer is used whenever the Host has a desktop to offer**, because most Owners give Orbite a machine of its own, and an agent that works where they can see it is the one they expect. A Host without a desktop (a server in a data centre) has only the second kind. The Owner chooses in plain words, or in the settings, and the Agent is told which one it is on.

## How the desktop is reached

A desktop belongs to the account that opened it, and no other account of the machine may look at it or act on it: not the Agent's, which is its own on purpose. So the desktop is offered from inside: a small helper, `orbite desk`, starts with the Owner's graphical session, as their account, and takes its orders from the Agent's service over a socket of the machine. Each side checks which account the other runs as; the installer writes down whose desktop may be offered, the account that installed Orbite. The helper looks at the screen, moves the pointer, types, and opens what is asked with the session's own programs.

Under X11 the helper does this directly. Under Wayland a program may only do it through the desktop's portal, which asks the Owner to allow it; that way is the next to be built, and until then a Wayland session offers nothing and the Agent keeps a computer of its own.

## Considered and not chosen

- **The Agent's service reaching the display itself**, with a key copied to it at login. It works under X11 only, leaves a key to the Owner's whole session in the Agent's folder, and gives no way to open something as the Owner.
- **Running Orbite as the Owner's account.** Everything the Agent runs would have the Owner's files and keys; the separate account is what keeps a mistaken command small.
- **A virtual keyboard and mouse made by the kernel** (uinput), with the screen read from the graphics card. It works on every desktop without asking anyone, which is the trouble: it takes root's rights for good, and it types keys rather than letters, wrong on any keyboard that is not the one assumed.

## Consequences

- On this computer the Agent acts with the Owner's signed-in accounts. The same Approvals apply as on a computer of its own, and the Owner sees every gesture; an Owner who wants distance says "use a computer of your own".
- The helper adds one program to the Owner's session and one file that starts it. Uninstalling removes both.
- Two people logged in on the same Host do not both offer their desktop: only the account written down at install does.
