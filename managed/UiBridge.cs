using System;
using System.Collections.Generic;
using System.IO;
using System.Text;
using System.Threading;
using System.Windows.Forms;

namespace FfxivActNative.Generated
{
    internal static class UiBridge
    {
        private const int MaxPayload = 1024 * 1024;
        private static readonly object gate = new object();
        private static readonly Queue<Command> commands = new Queue<Command>();
        private static readonly Queue<byte[]> events = new Queue<byte[]>();
        private static readonly Dictionary<uint, Control> controls = new Dictionary<uint, Control>();
        private static readonly Dictionary<uint, uint> logLimits = new Dictionary<uint, uint>();
        private static TabPage page;
        private static TableLayoutPanel root;
        private static bool accepting;
        private static bool started;
        private static bool commandScheduled;
        private static bool eventScheduled;
        private static bool applying;
        private static int activeCallbacks;

        internal static void Bind(TabPage value)
        {
            page = value;
            root = new TableLayoutPanel
            {
                AutoScroll = true,
                ColumnCount = 1,
                Dock = DockStyle.Fill,
            };
            root.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100));
            page.Controls.Add(root);
            lock (gate)
            {
                commands.Clear();
                events.Clear();
                accepting = true;
                started = false;
                commandScheduled = false;
                eventScheduled = false;
                activeCallbacks = 0;
            }
        }

        internal static void Queue(byte[] payload)
        {
            if (payload == null || payload.Length > MaxPayload)
                throw new ArgumentException("UI command payload");

            Command command = Command.Decode(payload);
            lock (gate)
            {
                if (!accepting)
                    throw new InvalidOperationException("UI unavailable");
                commands.Enqueue(command);
                ScheduleCommands();
            }
        }

        internal static void Start()
        {
            lock (gate)
            {
                started = true;
                ScheduleCommands();
            }
        }

        internal static void Stop()
        {
            lock (gate)
            {
                accepting = false;
                started = false;
                commands.Clear();
                events.Clear();
                while (activeCallbacks != 0)
                    Monitor.Wait(gate);
            }

            Control value = root;
            if (value == null)
                return;
            try
            {
                if (value.InvokeRequired)
                    value.Invoke(new Action(Cleanup));
                else
                    Cleanup();
            }
            catch { }
        }

        private static void ScheduleCommands()
        {
            if (!started || commandScheduled || commands.Count == 0 || root == null)
                return;
            commandScheduled = true;
            try
            {
                root.BeginInvoke(new Action(DrainCommands));
            }
            catch
            {
                commandScheduled = false;
                throw;
            }
        }

        private static void DrainCommands()
        {
            while (true)
            {
                Command command;
                lock (gate)
                {
                    if (!accepting || commands.Count == 0)
                    {
                        commandScheduled = false;
                        return;
                    }
                    command = commands.Dequeue();
                }

                try
                {
                    applying = true;
                    Apply(command);
                }
                catch (Exception ex)
                {
                    EmitError(command.HasId ? (uint?)command.Id : null, ex.Message);
                }
                finally { applying = false; }
            }
        }

        private static void Apply(Command command)
        {
            switch (command.Kind)
            {
                case 0:
                    Add(command);
                    break;
                case 1:
                    Remove(command.Id);
                    break;
                case 2:
                    Clear();
                    break;
                case 3:
                    SetText(command.Id, command.Text);
                    break;
                case 4:
                    Get(command.Id).Enabled = command.Bool;
                    break;
                case 5:
                    GetAs<CheckBox>(command.Id).Checked = command.Bool;
                    break;
                case 6:
                    SetItems(command);
                    break;
                case 7:
                    SetSelected(GetAs<ComboBox>(command.Id), command);
                    break;
                case 8:
                    SetNumber(command.Id, command.Value);
                    break;
                case 9:
                    AppendLog(command.Id, command.Text);
                    break;
                case 10:
                    GetAs<ListBox>(command.Id).Items.Clear();
                    break;
                default:
                    throw new ArgumentOutOfRangeException("command");
            }
        }

        private static void Add(Command command)
        {
            if (controls.ContainsKey(command.Id))
                throw new ArgumentException("duplicate control ID");

            Control parent = root;
            if (command.HasParent)
                parent = GetAs<FlowLayoutPanel>(command.Parent);

            Control control;
            uint id = command.Id;
            switch (command.ControlKind)
            {
                case 0:
                    control = new FlowLayoutPanel
                    {
                        AutoSize = true,
                        Dock = DockStyle.Fill,
                        FlowDirection = FlowDirection.LeftToRight,
                        WrapContents = true,
                    };
                    break;
                case 1:
                    control = new FlowLayoutPanel
                    {
                        AutoSize = true,
                        Dock = DockStyle.Fill,
                        FlowDirection = FlowDirection.TopDown,
                        WrapContents = false,
                    };
                    break;
                case 2:
                    control = new Label { AutoSize = true, Text = command.Text };
                    break;
                case 3:
                    Button button = new Button { AutoSize = true, Text = command.Text };
                    button.Click += delegate
                    {
                        EmitClicked(id);
                    };
                    control = button;
                    break;
                case 4:
                    CheckBox checkBox = new CheckBox { AutoSize = true, Text = command.Text, Checked = command.Bool };
                    checkBox.CheckedChanged += delegate
                    {
                        if (!applying)
                            EmitChecked(id, checkBox.Checked);
                    };
                    control = checkBox;
                    break;
                case 5:
                    TextBox textBox = new TextBox { Text = command.Text };
                    textBox.TextChanged += delegate
                    {
                        if (!applying)
                            EmitText(id, textBox.Text);
                    };
                    control = textBox;
                    break;
                case 6:
                    ComboBox comboBox = new ComboBox { DropDownStyle = ComboBoxStyle.DropDownList };
                    comboBox.Items.AddRange(command.Items.ToArray());
                    SetSelected(comboBox, command);
                    comboBox.SelectedIndexChanged += delegate
                    {
                        if (!applying)
                            EmitSelected(id, comboBox.SelectedIndex);
                    };
                    control = comboBox;
                    break;
                case 7:
                    NumericUpDown number = new NumericUpDown
                    {
                        Minimum = command.Min,
                        Maximum = command.Max,
                        Increment = command.Step,
                        Value = command.Value,
                    };
                    number.ValueChanged += delegate
                    {
                        if (!applying)
                            EmitNumber(id, Decimal.ToInt64(number.Value));
                    };
                    control = number;
                    break;
                case 8:
                    ListBox list = new ListBox { Height = 120 };
                    logLimits.Add(id, command.MaxRows);
                    control = list;
                    break;
                default:
                    throw new ArgumentOutOfRangeException("control");
            }

            control.Tag = id;
            if (parent == root)
                control.Dock = DockStyle.Top;
            else if (!(control is FlowLayoutPanel))
                control.Width = control is Label || control is Button || control is CheckBox ? control.PreferredSize.Width : 200;

            controls.Add(id, control);
            parent.Controls.Add(control);
            ReflowRoot();
        }

        private static void Remove(uint id)
        {
            Control control = Get(id);
            Control parent = control.Parent;
            Unregister(control);
            if (parent != null)
                parent.Controls.Remove(control);
            control.Dispose();
            ReflowRoot();
        }

        private static void Clear()
        {
            foreach (Control control in new List<Control>(controls.Values))
                if (control.Parent == root)
                    control.Dispose();
            controls.Clear();
            logLimits.Clear();
            root.Controls.Clear();
            root.RowStyles.Clear();
            root.RowCount = 0;
        }

        private static void Cleanup()
        {
            Clear();
            if (page != null && root != null)
                page.Controls.Remove(root);
            if (root != null)
                root.Dispose();
            root = null;
            page = null;
        }

        private static void Unregister(Control control)
        {
            foreach (Control child in control.Controls)
                Unregister(child);
            if (control.Tag is uint)
            {
                uint id = (uint)control.Tag;
                controls.Remove(id);
                logLimits.Remove(id);
            }
        }

        private static void ReflowRoot()
        {
            root.RowStyles.Clear();
            root.RowCount = root.Controls.Count;
            for (int row = 0; row < root.Controls.Count; row++)
            {
                root.SetRow(root.Controls[row], row);
                root.RowStyles.Add(new RowStyle(SizeType.AutoSize));
            }
        }

        private static Control Get(uint id)
        {
            Control control;
            if (!controls.TryGetValue(id, out control))
                throw new ArgumentException("unknown control ID");
            return control;
        }

        private static T GetAs<T>(uint id) where T : Control
        {
            T control = Get(id) as T;
            if (control == null)
                throw new ArgumentException("wrong control type");
            return control;
        }

        private static void SetText(uint id, string text)
        {
            Control control = Get(id);
            if (!(control is Label) && !(control is Button) && !(control is CheckBox) && !(control is TextBox))
                throw new ArgumentException("control has no settable text");
            control.Text = text;
        }

        private static void SetItems(Command command)
        {
            ComboBox control = GetAs<ComboBox>(command.Id);
            control.Items.Clear();
            control.Items.AddRange(command.Items.ToArray());
            SetSelected(control, command);
        }

        private static void SetSelected(ComboBox control, Command command)
        {
            if (command.HasSelected && command.Selected >= control.Items.Count)
                throw new ArgumentOutOfRangeException("selected");
            control.SelectedIndex = command.HasSelected ? checked((int)command.Selected) : -1;
        }

        private static void SetNumber(uint id, long value)
        {
            NumericUpDown control = GetAs<NumericUpDown>(id);
            if (value < control.Minimum || value > control.Maximum)
                throw new ArgumentOutOfRangeException("value");
            control.Value = value;
        }

        private static void AppendLog(uint id, string line)
        {
            ListBox control = GetAs<ListBox>(id);
            uint limit = logLimits[id];
            control.Items.Add(line);
            while (control.Items.Count > limit)
                control.Items.RemoveAt(0);
            control.TopIndex = control.Items.Count - 1;
        }

        private static void EmitClicked(uint id) { Emit(0, id, null); }
        private static void EmitChecked(uint id, bool value) { Emit(1, id, delegate(BinaryWriter w) { w.Write((byte)(value ? 1 : 0)); }); }
        private static void EmitText(uint id, string value) { Emit(2, id, delegate(BinaryWriter w) { WriteString(w, value); }); }
        private static void EmitSelected(uint id, int selected)
        {
            Emit(3, id, delegate(BinaryWriter w)
            {
                w.Write((byte)(selected >= 0 ? 1 : 0));
                if (selected >= 0)
                    w.Write((uint)selected);
            });
        }
        private static void EmitNumber(uint id, long value) { Emit(4, id, delegate(BinaryWriter w) { w.Write(value); }); }
        private static void EmitError(uint? id, string message)
        {
            byte[] payload;
            using (MemoryStream stream = new MemoryStream())
            using (BinaryWriter writer = new BinaryWriter(stream))
            {
                writer.Write((byte)5);
                writer.Write((byte)(id.HasValue ? 1 : 0));
                if (id.HasValue)
                    writer.Write(id.Value);
                WriteString(writer, message);
                payload = stream.ToArray();
            }
            EnqueueEvent(payload);
        }

        private static void Emit(byte kind, uint id, Action<BinaryWriter> write)
        {
            if (applying)
                return;
            byte[] payload;
            using (MemoryStream stream = new MemoryStream())
            using (BinaryWriter writer = new BinaryWriter(stream))
            {
                writer.Write(kind);
                writer.Write(id);
                if (write != null)
                    write(writer);
                payload = stream.ToArray();
            }
            EnqueueEvent(payload);
        }

        private static void EnqueueEvent(byte[] payload)
        {
            lock (gate)
            {
                if (!accepting || !started)
                    return;
                events.Enqueue(payload);
                if (eventScheduled)
                    return;
                eventScheduled = true;
                ThreadPool.QueueUserWorkItem(delegate { DrainEvents(); });
            }
        }

        private static void DrainEvents()
        {
            while (true)
            {
                byte[] payload;
                lock (gate)
                {
                    if (!accepting || events.Count == 0)
                    {
                        eventScheduled = false;
                        return;
                    }
                    payload = events.Dequeue();
                    activeCallbacks++;
                }
                try
                {
                    CommonBridge.SendRaw(11, payload);
                }
                finally
                {
                    lock (gate)
                    {
                        activeCallbacks--;
                        Monitor.PulseAll(gate);
                    }
                }
            }
        }

        private static void WriteString(BinaryWriter writer, string value)
        {
            byte[] bytes = Encoding.UTF8.GetBytes(value ?? "");
            writer.Write((uint)bytes.Length);
            writer.Write(bytes);
        }

        private sealed class Command
        {
            internal byte Kind;
            internal byte ControlKind;
            internal bool HasParent;
            internal uint Parent;
            internal uint Id;
            internal bool HasId;
            internal string Text;
            internal bool Bool;
            internal List<string> Items;
            internal bool HasSelected;
            internal uint Selected;
            internal long Min;
            internal long Max;
            internal long Step;
            internal long Value;
            internal uint MaxRows;

            internal static Command Decode(byte[] payload)
            {
                Command value = new Command { Items = new List<string>() };
                using (Reader reader = new Reader(payload))
                {
                    value.Kind = reader.Byte();
                    switch (value.Kind)
                    {
                        case 0:
                            value.HasParent = reader.Bool();
                            if (value.HasParent)
                                value.Parent = reader.UInt32();
                            value.ControlKind = reader.Byte();
                            value.Id = reader.UInt32();
                            value.HasId = true;
                            switch (value.ControlKind)
                            {
                                case 0:
                                case 1:
                                    break;
                                case 2:
                                case 3:
                                case 5:
                                    value.Text = reader.String();
                                    break;
                                case 4:
                                    value.Text = reader.String();
                                    value.Bool = reader.Bool();
                                    break;
                                case 6:
                                    value.Items = reader.Strings();
                                    ReadSelected(reader, value);
                                    break;
                                case 7:
                                    value.Min = reader.Int64();
                                    value.Max = reader.Int64();
                                    value.Step = reader.Int64();
                                    value.Value = reader.Int64();
                                    if (value.Min > value.Max
                                        || value.Step <= 0
                                        || value.Value < value.Min
                                        || value.Value > value.Max)
                                        throw new ArgumentOutOfRangeException("number");
                                    break;
                                case 8:
                                    value.MaxRows = reader.UInt32();
                                    if (value.MaxRows == 0 || value.MaxRows > 10000)
                                        throw new ArgumentOutOfRangeException("maxRows");
                                    break;
                                default:
                                    throw new ArgumentOutOfRangeException("control");
                            }
                            break;
                        case 1:
                        case 10:
                            value.Id = reader.UInt32();
                            value.HasId = true;
                            break;
                        case 2:
                            break;
                        case 3:
                        case 9:
                            value.Id = reader.UInt32();
                            value.HasId = true;
                            value.Text = reader.String();
                            break;
                        case 4:
                        case 5:
                            value.Id = reader.UInt32();
                            value.HasId = true;
                            value.Bool = reader.Bool();
                            break;
                        case 6:
                            value.Id = reader.UInt32();
                            value.HasId = true;
                            value.Items = reader.Strings();
                            ReadSelected(reader, value);
                            break;
                        case 7:
                            value.Id = reader.UInt32();
                            value.HasId = true;
                            ReadSelected(reader, value);
                            break;
                        case 8:
                            value.Id = reader.UInt32();
                            value.HasId = true;
                            value.Value = reader.Int64();
                            break;
                        default:
                            throw new ArgumentOutOfRangeException("command");
                    }
                    reader.End();
                }
                if (value.HasSelected && value.Selected >= value.Items.Count && (value.Kind == 0 || value.Kind == 6))
                    throw new ArgumentOutOfRangeException("selected");
                return value;
            }

            private static void ReadSelected(Reader reader, Command value)
            {
                value.HasSelected = reader.Bool();
                if (value.HasSelected)
                    value.Selected = reader.UInt32();
            }
        }

        private sealed class Reader : IDisposable
        {
            private static readonly UTF8Encoding Utf8 = new UTF8Encoding(false, true);
            private readonly MemoryStream stream;
            private readonly BinaryReader reader;

            internal Reader(byte[] payload)
            {
                stream = new MemoryStream(payload, false);
                reader = new BinaryReader(stream, Utf8);
            }
            internal byte Byte() { return reader.ReadByte(); }
            internal bool Bool()
            {
                byte value = Byte();
                if (value > 1)
                    throw new InvalidDataException("invalid boolean");
                return value != 0;
            }
            internal uint UInt32() { return reader.ReadUInt32(); }
            internal long Int64() { return reader.ReadInt64(); }
            internal string String()
            {
                uint length = UInt32();
                if (length > stream.Length - stream.Position)
                    throw new EndOfStreamException();
                return Utf8.GetString(reader.ReadBytes(checked((int)length)));
            }
            internal List<string> Strings()
            {
                uint count = UInt32();
                if (count > 10000)
                    throw new InvalidDataException("too many items");
                List<string> values = new List<string>(checked((int)count));
                for (uint i = 0; i < count; i++)
                    values.Add(String());
                return values;
            }
            internal void End()
            {
                if (stream.Position != stream.Length)
                    throw new InvalidDataException("trailing UI command data");
            }
            public void Dispose()
            {
                reader.Dispose();
                stream.Dispose();
            }
        }
    }
}
