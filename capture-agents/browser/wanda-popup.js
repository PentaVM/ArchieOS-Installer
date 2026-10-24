const wandaUrlField = document.getElementById('wanda-url');
const wandaTokenField = document.getElementById('wanda-token');
const wandaEnabledField = document.getElementById('wanda-enabled');
const wandaPresumedField = document.getElementById('wanda-presumed');
const wandaStatus = document.getElementById('wanda-status');
chrome.storage.local.get(['wandaUrl', 'wandaToken', 'wandaEnabled', 'wandaAllowPresumed', 'wandaSuspended']).then(config => {
  wandaUrlField.value = config.wandaUrl || '';
  wandaTokenField.value = config.wandaToken || '';
  wandaEnabledField.checked = config.wandaEnabled === true;
  wandaPresumedField.checked = config.wandaAllowPresumed === true;
  wandaStatus.textContent = config.wandaSuspended ? 'Content suspended for another user.' : 'Visible-chat coverage only; source tokens fix the domain on your node.';
});
document.getElementById('wanda-save').addEventListener('click', async () => {
  const endpoint = wandaUrlField.value.trim();
  if (!WandaTransport.endpoint(endpoint)) { wandaStatus.textContent = 'Use HTTPS, or HTTP loopback, with the exact /v1/events path.'; return; }
  if (wandaEnabledField.checked && !wandaTokenField.value.trim()) { wandaStatus.textContent = 'Enter the Wanda source token before enabling content.'; return; }
  const config = await chrome.storage.local.get(['deviceId']);
  await chrome.storage.local.set({ deviceId: config.deviceId || 'browser-' + crypto.randomUUID().slice(0, 12),
    wandaUrl: endpoint, wandaToken: wandaTokenField.value.trim(), wandaEnabled: wandaEnabledField.checked,
    wandaAllowPresumed: wandaPresumedField.checked });
  wandaStatus.textContent = 'Wanda pairing saved. Enroll this device ID on your node: ' + (await chrome.storage.local.get('deviceId')).deviceId;
});
document.getElementById('wanda-suspend').addEventListener('click', async () => {
  await chrome.storage.local.set({ wandaSuspended: true });
  wandaStatus.textContent = 'Content suspended; queued bodies cleared.';
});
document.getElementById('wanda-resume').addEventListener('click', async () => {
  await chrome.storage.local.set({ wandaSuspended: false });
  wandaStatus.textContent = 'Resumed with presumed-owner attribution if content and that attribution are enabled.';
});
