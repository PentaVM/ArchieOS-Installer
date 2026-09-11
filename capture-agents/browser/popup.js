// Pairing: store the node's capture URL and token. The extension talks only to this node.
const urlField = document.getElementById("url");
const tokenField = document.getElementById("token");
const status = document.getElementById("status");

chrome.storage.local.get(["ingestUrl", "token"]).then((s) => {
  urlField.value = s.ingestUrl || "";
  tokenField.value = s.token || "";
});

document.getElementById("save").addEventListener("click", async () => {
  await chrome.storage.local.set({
    ingestUrl: urlField.value.trim(),
    token: tokenField.value.trim(),
  });
  status.textContent = "Paired. Your searches and navigation now go to your node.";
});
