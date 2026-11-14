# SPDX-FileCopyrightText: Copyright (c) 2026 Donald Nadeau Jr. (SyberShot) <https://xiarv0.com>
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional term (AGPLv3 s.7(b)): see LICENSE-ADDITIONAL-TERMS.md
# Registers the NeuralBrain host tasks for the CURRENT user (no admin, no UAC):
#   XIAR-NeuralBrain-Service  at logon: brain query API (127.0.0.1, config [service])
#   XIAR-NeuralBrain-Trainer  every [train].check_interval_minutes: idle-window trainer (skips if GPU busy)
#   XIAR-NeuralBrain-Tunnel   at logon: optional reverse SSH tunnel so another machine can reach the API
# Re-run any time; -Remove unregisters them.
param(
  [string]$Root = (Resolve-Path "$PSScriptRoot\..\..\..").Path,
  [switch]$Remove,
  [switch]$NoTunnel
)
$names = 'XIAR-NeuralBrain-Service','XIAR-NeuralBrain-Trainer','XIAR-NeuralBrain-Tunnel'
foreach ($n in $names) { Unregister-ScheduledTask -TaskName $n -Confirm:$false -ErrorAction SilentlyContinue }
if ($Remove) { 'removed'; return }
$pkg = Join-Path $Root 'core\neural-brain'
$py = Join-Path $pkg '.venv\Scripts\pythonw.exe'
$cfg = Join-Path $Root 'config\neural_brain.toml'
$m = Select-String -LiteralPath $cfg -Pattern '^\s*check_interval_minutes\s*=\s*(\d+)' | Select -First 1
$interval = if ($m) { [int]$m.Matches[0].Groups[1].Value } else { 30 }
$user = [Security.Principal.WindowsIdentity]::GetCurrent().Name
$principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable -MultipleInstances IgnoreNew -ExecutionTimeLimit (New-TimeSpan -Days 3650)
$logon = New-ScheduledTaskTrigger -AtLogOn -User $user
$a1 = New-ScheduledTaskAction -Execute $py -Argument "-m xiar_brain --config `"$cfg`" serve" -WorkingDirectory $pkg
Register-ScheduledTask -TaskName $names[0] -Action $a1 -Trigger $logon -Principal $principal -Settings $settings -Description 'XIAR NeuralBrain query API (localhost)' | Out-Null
$rep = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(2) -RepetitionInterval (New-TimeSpan -Minutes $interval)
$tset = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -StartWhenAvailable -MultipleInstances IgnoreNew -ExecutionTimeLimit (New-TimeSpan -Hours 2)
$a2 = New-ScheduledTaskAction -Execute $py -Argument "-m xiar_brain --config `"$cfg`" train" -WorkingDirectory $pkg
Register-ScheduledTask -TaskName $names[1] -Action $a2 -Trigger $rep -Principal $principal -Settings $tset -Description 'XIAR NeuralBrain idle-window trainer' | Out-Null
if (-not $NoTunnel -and (Test-Path "$PSScriptRoot\brain-tunnel.ps1")) {
  $a3 = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$PSScriptRoot\brain-tunnel.ps1`" -Root `"$Root`"" -WorkingDirectory $Root
  Register-ScheduledTask -TaskName $names[2] -Action $a3 -Trigger $logon -Principal $principal -Settings $settings -Description 'XIAR NeuralBrain optional reverse SSH tunnel to another machine' | Out-Null
}
Get-ScheduledTask -TaskName 'XIAR-NeuralBrain-*' | Select TaskName,State