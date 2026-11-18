# The Agent's Computer is mandatory

Every installation gives the Agent a Computer: a real desktop with a screen and a browser on the Host. It is not an optional component, even though it roughly doubles the memory a Host needs and rules out the smallest machines: the Computer is what lets the Agent work with services that have no API, and an Orbite without it would be a different, weaker product. Installation warns when a Host is too small rather than offering to skip it. For reading the web, the Agent uses fast page fetching first and the browser only when needed.
