# Troubleshooting

## Common Issues

### Bootstrap fails with "authentication failed"

1. Verify your PAT has the required scopes
2. Check that the PAT hasn't expired
3. Ensure the host URL is correct (include `https://` for self-hosted instances)
4. Try re-entering the PAT in Provider settings

### Clone hangs or times out

- Large repositories may take time. Check the project's compute target has adequate network access.
- If using SSH, verify the host key is accepted.
- Check your network connection and firewall settings.

### Feature pipeline gets stuck

1. Check the **Gate** — human approval may be required (look for the amber glow)
2. Verify the agent machine is running and reachable
3. Check the step logs for error messages
4. Use **Cancel Feature** and retry with a different model or agent

### Merge conflict gate won't resolve

- The conflict viewer shows each conflicting file. Provide specific instructions for each file.
- If the auto-agent resolver failed, switch to manual resolution.
- For complex conflicts, resolve them directly in your IDE and push the fix to the feature branch.

### "Machine not found" error

1. Open **Settings → Machines** and verify the machine exists
2. Test the connection to confirm credentials are valid
3. If the machine was deleted, create a new one or switch the project to local compute

## Getting Help

If you encounter an issue not covered here, check the project logs:

```bash
# Tauri backend logs
~/.demeteo/logs/

# Feature execution traces
~/.demeteo/artifacts/<feature-id>/
```
