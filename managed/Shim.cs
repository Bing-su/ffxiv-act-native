using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Windows.Forms;
using Advanced_Combat_Tracker;
using FFXIV_ACT_Plugin.Common;
using FFXIV_ACT_Plugin.Common.Models;

namespace ActBridge.Generated
{
    [StructLayout(LayoutKind.Sequential)]
    internal struct HostApi
    {
        internal uint Version;
        internal IntPtr Context;
        [MarshalAs(UnmanagedType.FunctionPtr)] internal QueryDelegate Query;
        [MarshalAs(UnmanagedType.FunctionPtr)] internal StatusDelegate Status;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct ClientApi
    {
        internal uint Version;
        internal uint Subscriptions;
        internal IntPtr Context;
        [MarshalAs(UnmanagedType.FunctionPtr)] internal EventDelegate Event;
        [MarshalAs(UnmanagedType.FunctionPtr)] internal ShutdownDelegate Shutdown;
    }

    [StructLayout(LayoutKind.Sequential)]
    internal struct RawEvent
    {
        internal uint Kind;
        internal IntPtr Payload;
        internal UIntPtr Length;
    }

    [UnmanagedFunctionPointer(CallingConvention.Winapi)]
    internal delegate int QueryDelegate(IntPtr context, uint query, IntPtr request,
        UIntPtr requestLength, IntPtr output, UIntPtr outputLength, out UIntPtr required);
    [UnmanagedFunctionPointer(CallingConvention.Winapi)]
    internal delegate void StatusDelegate(IntPtr context, IntPtr message, UIntPtr length);
    [UnmanagedFunctionPointer(CallingConvention.Winapi)]
    internal delegate int EventDelegate(IntPtr context, ref RawEvent value);
    [UnmanagedFunctionPointer(CallingConvention.Winapi)]
    internal delegate int ShutdownDelegate(IntPtr context);

    internal static class CommonBridge
    {
        private static IDataSubscription subscription;
        private static IDataRepository repository;
        private static ClientApi client;
        private static readonly object callbackGate = new object();
        private static bool stopping = true;

        internal static void Bind(object subscriptions, object repositoryValue)
        {
            subscription = (IDataSubscription)subscriptions;
            repository = (IDataRepository)repositoryValue;
        }

        internal static void Subscribe(ClientApi value)
        {
            lock (callbackGate)
            {
                client = value;
                stopping = false;
            }

            uint s = value.Subscriptions;
            if ((s & (1u << 0)) != 0) subscription.NetworkReceived += NetworkReceived;
            if ((s & (1u << 1)) != 0) subscription.NetworkSent += NetworkSent;
            if ((s & (1u << 2)) != 0) subscription.CombatantAdded += CombatantAdded;
            if ((s & (1u << 3)) != 0) subscription.CombatantRemoved += CombatantRemoved;
            if ((s & (1u << 4)) != 0) subscription.PrimaryPlayerChanged += PrimaryPlayerChanged;
            if ((s & (1u << 5)) != 0) subscription.ZoneChanged += ZoneChanged;
            if ((s & (1u << 6)) != 0) subscription.PlayerStatsChanged += PlayerStatsChanged;
            if ((s & (1u << 7)) != 0) subscription.PartyListChanged += PartyListChanged;
            if ((s & (1u << 8)) != 0) subscription.LogLine += LogLine;
            if ((s & (1u << 9)) != 0) subscription.ParsedLogLine += ParsedLogLine;
            if ((s & (1u << 10)) != 0) subscription.ProcessChanged += ProcessChanged;
        }

        internal static void Unsubscribe()
        {
            lock (callbackGate)
            {
                stopping = true;
            }

            if (subscription != null)
            {
                subscription.NetworkReceived -= NetworkReceived;
                subscription.NetworkSent -= NetworkSent;
                subscription.CombatantAdded -= CombatantAdded;
                subscription.CombatantRemoved -= CombatantRemoved;
                subscription.PrimaryPlayerChanged -= PrimaryPlayerChanged;
                subscription.ZoneChanged -= ZoneChanged;
                subscription.PlayerStatsChanged -= PlayerStatsChanged;
                subscription.PartyListChanged -= PartyListChanged;
                subscription.LogLine -= LogLine;
                subscription.ParsedLogLine -= ParsedLogLine;
                subscription.ProcessChanged -= ProcessChanged;
            }

            subscription = null;
        }

        internal static void Stop()
        {
            Unsubscribe();
            repository = null;
            lock (callbackGate)
            {
                client = default(ClientApi);
            }
        }

        internal static int Query(IntPtr context, uint query, IntPtr request,
            UIntPtr requestLength, IntPtr output, UIntPtr outputLength, out UIntPtr required)
        {
            try
            {
                byte[] result = QueryCore(query, request, checked((int)requestLength.ToUInt64()));
                required = new UIntPtr((uint)result.Length);
                if (output == IntPtr.Zero || outputLength.ToUInt64() < (ulong)result.Length)
                    return 6;
                if (result.Length != 0)
                    Marshal.Copy(result, 0, output, result.Length);

                return 0;
            }
            catch (Exception ex)
            {
                required = UIntPtr.Zero;
                Plugin.SetStatus("Rust bridge repository error: " + ex.Message);
                return 4;
            }
        }

        private static byte[] QueryCore(uint query, IntPtr request, int requestLength)
        {
            if (repository == null)
                throw new InvalidOperationException("repository unavailable");

            using (MemoryStream stream = new MemoryStream())
            using (BinaryWriter writer = new BinaryWriter(stream))
            {
                switch (query)
                {
                    case 1:
                        writer.Write(repository.GetCurrentPlayerID());
                        break;
                    case 2:
                        writer.Write(repository.GetCurrentTerritoryID());
                        break;
                    case 3:
                        writer.Write((int)repository.GetSelectedLanguageID());
                        break;
                    case 4:
                        WriteString(writer, repository.GetGameVersion());
                        break;
                    case 5:
                        writer.Write(repository.IsChatLogAvailable());
                        break;
                    case 6:
                        var combatants = repository.GetCombatantList();
                        writer.Write((uint)combatants.Count);
                        foreach (Combatant combatant in combatants)
                            WriteCombatant(writer, combatant);
                        break;
                    case 7:
                        WritePlayer(writer, repository.GetPlayer());
                        break;
                    case 8:
                        if (requestLength != 4)
                            throw new ArgumentException("resource query payload");

                        byte[] kind = new byte[4];
                        Marshal.Copy(request, kind, 0, 4);
                        IDictionary<uint, string> resources = repository.GetResourceDictionary(
                            (ResourceType)BitConverter.ToInt32(kind, 0));
                        writer.Write((uint)resources.Count);
                        foreach (KeyValuePair<uint, string> pair in resources)
                        {
                            writer.Write(pair.Key);
                            WriteString(writer, pair.Value);
                        }
                        break;
                    case 9:
                        Process process = repository.GetCurrentFFXIVProcess();
                        writer.Write(process == null ? 0u : (uint)process.Id);
                        break;
                    case 10:
                        writer.Write(repository.GetServerTimestamp().Ticks);
                        break;
                    case 11:
                        string[] names = repository.GetAntiVirusNames() ?? new string[0];
                        writer.Write((uint)names.Length);
                        foreach (string name in names)
                            WriteString(writer, name);
                        break;
                    case 12:
                        writer.Write(repository.GetGameRegion());
                        break;
                    default:
                        throw new ArgumentOutOfRangeException("query");
                }
                return stream.ToArray();
            }
        }

        private static void Send(uint kind, Action<BinaryWriter> write)
        {
            try
            {
                byte[] bytes;
                using (MemoryStream stream = new MemoryStream())
                using (BinaryWriter writer = new BinaryWriter(stream))
                {
                    write(writer);
                    bytes = stream.ToArray();
                }

                GCHandle pin = default(GCHandle);
                try
                {
                    RawEvent value = new RawEvent { Kind = kind, Length = new UIntPtr((uint)bytes.Length) };
                    if (bytes.Length != 0)
                    {
                        pin = GCHandle.Alloc(bytes, GCHandleType.Pinned);
                        value.Payload = pin.AddrOfPinnedObject();
                    }

                    lock (callbackGate)
                    {
                        if (stopping || client.Event == null)
                            return;
                        if (client.Event(client.Context, ref value) != 0)
                            Plugin.SetStatus("Rust bridge: event callback failed");
                    }
                }
                finally
                {
                    if (pin.IsAllocated)
                        pin.Free();
                }
            }
            catch (Exception ex)
            {
                Plugin.SetStatus("Rust bridge event error: " + ex.Message);
            }
        }

        private static void NetworkReceived(string connection, long timestamp, byte[] bytes)
        {
            Send(0, writer =>
            {
                writer.Write(timestamp);
                WriteString(writer, connection);
                WriteBytes(writer, bytes);
            });
        }

        private static void NetworkSent(string connection, long timestamp, byte[] bytes)
        {
            Send(1, writer =>
            {
                writer.Write(timestamp);
                WriteString(writer, connection);
                WriteBytes(writer, bytes);
            });
        }

        private static void CombatantAdded(object value) { Send(2, w => WriteCombatant(w, (Combatant)value)); }
        private static void CombatantRemoved(object value) { Send(3, w => WriteCombatant(w, (Combatant)value)); }
        private static void PrimaryPlayerChanged() { Send(4, w => { }); }
        private static void ZoneChanged(uint id, string name)
        {
            Send(5, writer =>
            {
                writer.Write(id);
                WriteString(writer, name);
            });
        }

        private static void PlayerStatsChanged(object value) { Send(6, w => WritePlayer(w, (Player)value)); }
        private static void PartyListChanged(ReadOnlyCollection<uint> ids, int size)
        {
            Send(7, writer =>
            {
                writer.Write(size);
                writer.Write((uint)ids.Count);
                foreach (uint id in ids)
                    writer.Write(id);
            });
        }

        private static void LogLine(uint type, uint seconds, string line)
        {
            Send(8, writer =>
            {
                writer.Write(type);
                writer.Write(seconds);
                WriteString(writer, line);
            });
        }

        private static void ParsedLogLine(uint type, int seconds, string line)
        {
            Send(9, writer =>
            {
                writer.Write(type);
                writer.Write(seconds);
                WriteString(writer, line);
            });
        }

        private static void ProcessChanged(Process process)
        {
            Send(10, writer => writer.Write(process == null ? 0u : (uint)process.Id));
        }

        private static void WriteBytes(BinaryWriter writer, byte[] value)
        {
            value = value ?? new byte[0];
            writer.Write((uint)value.Length);
            writer.Write(value);
        }

        private static void WriteString(BinaryWriter writer, string value)
        {
            WriteBytes(writer, Encoding.UTF8.GetBytes(value ?? string.Empty));
        }

        private static void WritePlayer(BinaryWriter writer, Player player)
        {
            if (player == null)
            {
                writer.Write(false);
                return;
            }

            writer.Write(true);
            writer.Write(player.JobID);
            writer.Write(player.Str);
            writer.Write(player.Dex);
            writer.Write(player.Vit);
            writer.Write(player.Intel);
            writer.Write(player.Mnd);
            writer.Write(player.Pie);
            writer.Write(player.Attack);
            writer.Write(player.DirectHit);
            writer.Write(player.Crit);
            writer.Write(player.AttackMagicPotency);
            writer.Write(player.HealMagicPotency);
            writer.Write(player.Det);
            writer.Write(player.SkillSpeed);
            writer.Write(player.SpellSpeed);
            writer.Write(player.Tenacity);
            writer.Write(player.LocalContentId);
        }

        private static void WriteCombatant(BinaryWriter writer, Combatant combatant)
        {
            if (combatant == null)
            {
                writer.Write(false);
                return;
            }

            writer.Write(true);
            writer.Write(combatant.ID);
            writer.Write(combatant.OwnerID);
            writer.Write(combatant.type);
            writer.Write(combatant.Job);
            writer.Write(combatant.Level);
            WriteString(writer, combatant.Name);
            writer.Write(combatant.CurrentHP);
            writer.Write(combatant.MaxHP);
            writer.Write(combatant.CurrentMP);
            writer.Write(combatant.MaxMP);
            writer.Write(combatant.CurrentCP);
            writer.Write(combatant.MaxCP);
            writer.Write(combatant.CurrentGP);
            writer.Write(combatant.MaxGP);
            writer.Write(combatant.IsCasting);
            writer.Write(combatant.CastBuffID);
            writer.Write(combatant.CastTargetID);
            writer.Write(combatant.CastDurationCurrent);
            writer.Write(combatant.CastDurationMax);
            writer.Write(combatant.PosX);
            writer.Write(combatant.PosY);
            writer.Write(combatant.PosZ);
            writer.Write(combatant.Heading);
            writer.Write(combatant.CurrentWorldID);
            writer.Write(combatant.WorldID);
            WriteString(writer, combatant.WorldName);
            writer.Write(combatant.BNpcNameID);
            writer.Write(combatant.BNpcID);
            writer.Write(combatant.TargetID);
            writer.Write(combatant.EffectiveDistance);
            writer.Write((int)combatant.PartyType);
            writer.Write(combatant.Address.ToInt64());
            writer.Write(combatant.Order);

            NetworkBuff[] buffs = combatant.NetworkBuffs ?? new NetworkBuff[0];
            writer.Write((uint)buffs.Length);
            foreach (NetworkBuff buff in buffs)
            {
                if (buff == null)
                {
                    writer.Write(false);
                    continue;
                }

                writer.Write(true);
                writer.Write(buff.BuffID);
                writer.Write(buff.BuffExtra);
                writer.Write(buff.Timestamp.Ticks);
                writer.Write(buff.Duration);
                writer.Write(buff.ActorID);
                WriteString(writer, buff.ActorName);
                writer.Write(buff.TargetID);
                WriteString(writer, buff.TargetName);
            }
        }
    }

    public sealed class Plugin : IActPluginV1
    {
        private static Label statusLabel;
        private static IntPtr nativeHandle;
        private static QueryDelegate query;
        private static StatusDelegate status;
        private static ClientApi client;

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        private static extern IntPtr LoadLibrary(string path);

        [DllImport("kernel32.dll")]
        private static extern bool FreeLibrary(IntPtr module);

        [DllImport("__ACT_BRIDGE_NATIVE__.dll", CallingConvention = CallingConvention.Winapi)]
        private static extern int act_bridge_entry_v1(ref HostApi host, out ClientApi client);

        public void InitPlugin(TabPage pluginScreenSpace, Label pluginStatusText)
        {
            statusLabel = pluginStatusText;
            try
            {
                object subscriptions, repository;
                if (!FindServices(out subscriptions, out repository))
                {
                    SetStatus("Rust bridge: enable FFXIV_ACT_Plugin first");
                    return;
                }

                CommonBridge.Bind(subscriptions, repository);
                string path = Path.Combine(
                    Path.GetDirectoryName(typeof(Plugin).Assembly.Location),
                    "__ACT_BRIDGE_NATIVE__.dll");
                nativeHandle = LoadLibrary(path);
                if (nativeHandle == IntPtr.Zero)
                    throw new InvalidOperationException("native DLL load failed");

                query = CommonBridge.Query;
                status = SetStatusNative;
                HostApi host = new HostApi { Version = 1, Query = query, Status = status };
                int result = act_bridge_entry_v1(ref host, out client);
                if (result != 0)
                    throw new InvalidOperationException("native init status " + result);
                if (client.Version != 1 || client.Event == null || client.Shutdown == null)
                    throw new InvalidOperationException("invalid native ABI");

                CommonBridge.Subscribe(client);
                SetStatus("Rust bridge loaded");
            }
            catch (Exception ex)
            {
                CommonBridge.Stop();
                Unload();
                SetStatus("Rust bridge: " + ex.Message);
            }
        }

        public void DeInitPlugin()
        {
            CommonBridge.Unsubscribe();
            try
            {
                if (client.Shutdown != null)
                    client.Shutdown(client.Context);
            }
            catch (Exception ex)
            {
                SetStatus("Rust bridge shutdown error: " + ex.Message);
            }
            finally
            {
                CommonBridge.Stop();
                client = default(ClientApi);
                query = null;
                status = null;
                Unload();
            }
        }

        private static void Unload()
        {
            if (nativeHandle == IntPtr.Zero)
                return;

            FreeLibrary(nativeHandle);
            nativeHandle = IntPtr.Zero;
        }

        private static bool FindServices(out object subscriptions, out object repository)
        {
            subscriptions = null;
            repository = null;
            foreach (ActPluginData item in ActGlobals.oFormActMain.ActPlugins)
            {
                if (!string.Equals(
                        item.pluginFile.Name,
                        "FFXIV_ACT_Plugin.dll",
                        StringComparison.OrdinalIgnoreCase)
                    || item.pluginObj == null)
                    continue;

                Type type = item.pluginObj.GetType();
                var started = type.GetProperty("PluginStarted");
                var dataSubscription = type.GetProperty("DataSubscription");
                var dataRepository = type.GetProperty("DataRepository");
                if (started == null
                    || dataSubscription == null
                    || dataRepository == null
                    || !(bool)started.GetValue(item.pluginObj, null))
                    return false;

                subscriptions = dataSubscription.GetValue(item.pluginObj, null);
                repository = dataRepository.GetValue(item.pluginObj, null);
                return subscriptions != null && repository != null;
            }

            return false;
        }

        private static void SetStatusNative(IntPtr context, IntPtr message, UIntPtr length)
        {
            try
            {
                int count = checked((int)length.ToUInt64());
                byte[] bytes = new byte[count];
                if (count != 0)
                    Marshal.Copy(message, bytes, 0, count);
                SetStatus(Encoding.UTF8.GetString(bytes));
            }
            catch { }
        }

        internal static void SetStatus(string text)
        {
            Label label = statusLabel;
            if (label == null || label.IsDisposed)
                return;

            if (label.InvokeRequired)
            {
                try { label.BeginInvoke(new Action<string>(SetStatus), text); }
                catch { }
                return;
            }

            label.Text = text;
        }
    }
}
