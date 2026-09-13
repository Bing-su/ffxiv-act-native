using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.Diagnostics;
using System.Reflection;

[assembly: AssemblyVersion("3.1.2.4")]

namespace FFXIV_ACT_Plugin.Common
{
    public enum Language { English }
    public enum ResourceType { Action }

    public delegate void NetworkReceivedDelegate(string connection, long timestamp, byte[] data);
    public delegate void NetworkSentDelegate(string connection, long timestamp, byte[] data);
    public delegate void CombatantAddedDelegate(object value);
    public delegate void CombatantRemovedDelegate(object value);
    public delegate void PrimaryPlayerChangedDelegate();
    public delegate void ZoneChangedDelegate(uint id, string name);
    public delegate void PlayerStatsChangedDelegate(object value);
    public delegate void PartyListChangedDelegate(ReadOnlyCollection<uint> ids, int size);
    public delegate void LogLineDelegate(uint type, uint seconds, string line);
    public delegate void ParsedLogLineDelegate(uint type, int seconds, string line);
    public delegate void ProcessChangedDelegate(Process process);

    public interface IDataSubscription
    {
        event NetworkReceivedDelegate NetworkReceived;
        event NetworkSentDelegate NetworkSent;
        event CombatantAddedDelegate CombatantAdded;
        event CombatantRemovedDelegate CombatantRemoved;
        event PrimaryPlayerChangedDelegate PrimaryPlayerChanged;
        event ZoneChangedDelegate ZoneChanged;
        event PlayerStatsChangedDelegate PlayerStatsChanged;
        event PartyListChangedDelegate PartyListChanged;
        event LogLineDelegate LogLine;
        event ParsedLogLineDelegate ParsedLogLine;
        event ProcessChangedDelegate ProcessChanged;
    }

    public interface IDataRepository
    {
        Language GetSelectedLanguageID();
        Process GetCurrentFFXIVProcess();
        IDictionary<uint, string> GetResourceDictionary(ResourceType type);
        uint GetCurrentTerritoryID();
        uint GetCurrentPlayerID();
        ReadOnlyCollection<Models.Combatant> GetCombatantList();
        Models.Player GetPlayer();
        DateTime GetServerTimestamp();
        string GetGameVersion();
        bool IsChatLogAvailable();
        string[] GetAntiVirusNames();
        byte GetGameRegion();
    }
}

namespace FFXIV_ACT_Plugin.Common.Models
{
    public class Combatant { }
    public class NetworkBuff { }
    public class Player { }
}
