export const REPO = "https://github.com/julioacontreras/my-graph";
const blob = `${REPO}/blob/main`;

export const links = {
  repo: REPO,
  issues: `${REPO}/issues`,
  pulls: `${REPO}/pulls`,
  changelog: `${blob}/CHANGELOG.md`,
  protocol: `${blob}/PROTOCOL.md`,
  license: `${blob}/LICENSE`,
  composeFile: `${blob}/docker-compose.yml`,
  readme: `${REPO}#readme`,
  status: `${REPO}#status`,
  quickRef: `${REPO}#reference`,
  cypher: `${REPO}#cypher-examples`,
  indexes: `${REPO}#indexes-you-create-them`,
  sessions: `${REPO}#sessions-databases-and-scripts`,
  resultFormat: `${REPO}#result-format`,
  usage: `${REPO}#usage`,
  workspace: `${REPO}#architecture`,
  designDecisions: `${REPO}#design-decisions-v01`,
  python: `${blob}/python/README.md`,
  dockerHub: "https://hub.docker.com/r/julioacontreras/mygraph",
  ghcr: "https://github.com/julioacontreras/my-graph/pkgs/container/mygraph",
  pypi: "https://pypi.org/project/mygraph-client/",
  llms: `${import.meta.env.BASE_URL}llms.txt`,
  llmsFull: `${import.meta.env.BASE_URL}llms-full.txt`,
} as const;
