using System;
using System.Collections.Generic;
using System.IO;
using System.Text;
using System.Threading;
using System.Windows.Forms;

namespace FfxivActNative.Generated
{
    internal static class CommonBridge
    {
        internal static readonly List<byte[]> Events = new List<byte[]>();
        internal static void SendRaw(uint kind, byte[] payload)
        {
            if (kind != 11)
                throw new InvalidOperationException("wrong event kind");
            lock (Events)
                Events.Add(payload);
        }
    }

    internal static class UiBridgeCheck
    {
        [STAThread]
        private static void Main()
        {
            try
            {
                Run();
            }
            catch (Exception ex)
            {
                Console.Error.WriteLine(ex.GetType().FullName + ": " + ex.Message);
                Environment.ExitCode = 1;
            }
        }

        private static void Run()
        {
            using (Form form = new Form())
            using (TabControl tabs = new TabControl())
            using (TabPage page = new TabPage())
            {
                tabs.TabPages.Add(page);
                form.Controls.Add(tabs);
                form.Show();
                UiBridge.Bind(page);
                UiBridge.Queue(AddRow(1));
                UiBridge.Queue(AddCheckBox(1, 2, "Enabled"));
                Assert(page.Controls[0].Controls.Count == 0, "init commands applied before start");
                UiBridge.Start();
                Wait(delegate { return page.Controls[0].Controls.Count == 1; });

                FlowLayoutPanel row = (FlowLayoutPanel)page.Controls[0].Controls[0];
                CheckBox check = (CheckBox)row.Controls[0];
                UiBridge.Queue(AddColumn(7));
                UiBridge.Queue(AddCheckBox(7, 8, "Narrow"));
                Wait(delegate { return page.Controls[0].Controls.Count == 2; });
                FlowLayoutPanel column = (FlowLayoutPanel)page.Controls[0].Controls[1];
                Assert(column.FlowDirection == FlowDirection.TopDown && !column.WrapContents,
                    "column layout is not vertical");
                Assert(column.Controls[0].Width < column.Width, "column child filled the tab width");
                UiBridge.Queue(SetChecked(2, true));
                Wait(delegate { return check.Checked; });
                Thread.Sleep(20);
                Assert(EventCount() == 0, "programmatic update emitted an event");

                check.Checked = false;
                Wait(delegate { return EventCount() == 1; });
                Assert(CommonBridge.Events[0][0] == 1, "wrong checkbox event");

                UiBridge.Queue(AddLog(3, 2));
                UiBridge.Queue(AppendLog(3, "one"));
                UiBridge.Queue(AppendLog(3, "two"));
                UiBridge.Queue(AppendLog(3, "three"));
                Wait(delegate { return page.Controls[0].Controls.Count == 3; });
                ListBox log = (ListBox)page.Controls[0].Controls[2];
                Wait(delegate { return log.Items.Count == 2; });
                Assert((string)log.Items[0] == "two", "log limit did not remove oldest row");

                UiBridge.Queue(AddComboBox(4));
                UiBridge.Queue(SetItems(4));
                UiBridge.Queue(AddNumber(5));
                UiBridge.Queue(SetNumber(5, 7));
                Wait(delegate { return page.Controls[0].Controls.Count == 5; });
                ComboBox combo = (ComboBox)page.Controls[0].Controls[3];
                NumericUpDown number = (NumericUpDown)page.Controls[0].Controls[4];
                Wait(delegate { return combo.Items.Count == 2 && combo.SelectedIndex == 1 && number.Value == 7; });
                Assert(EventCount() == 1, "programmatic selection or number update emitted an event");

                UiBridge.Queue(AddLabel(999, 6));
                Wait(delegate { return EventCount() == 2; });
                UiBridge.Queue(AddRow(1));
                Wait(delegate { return EventCount() == 3; });
                Assert(CommonBridge.Events[1][0] == 5 && CommonBridge.Events[2][0] == 5,
                    "invalid hierarchy or duplicate ID did not emit an error");

                bool malformed = false;
                try
                {
                    UiBridge.Queue(new byte[] { 99 });
                }
                catch (ArgumentOutOfRangeException)
                {
                    malformed = true;
                }
                Assert(malformed, "malformed command was accepted");

                UiBridge.Queue(Remove(3));
                Wait(delegate { return page.Controls[0].Controls.Count == 4; });
                UiBridge.Stop();
                Assert(page.Controls.Count == 0, "UI was not removed on stop");
            }
        }

        private static int EventCount()
        {
            lock (CommonBridge.Events)
                return CommonBridge.Events.Count;
        }
        private static void Wait(Func<bool> condition)
        {
            for (int i = 0; i < 100 && !condition(); i++)
            {
                Application.DoEvents();
                Thread.Sleep(5);
            }
            Assert(condition(), "timed out");
        }
        private static void Assert(bool condition, string message)
        {
            if (!condition)
                throw new InvalidOperationException(message);
        }

        private static byte[] AddRow(uint id)
        {
            return Write(delegate(BinaryWriter writer)
            {
                writer.Write((byte)0);
                writer.Write((byte)0);
                writer.Write((byte)0);
                writer.Write(id);
            });
        }
        private static byte[] AddColumn(uint id)
        {
            return Write(delegate(BinaryWriter writer)
            {
                writer.Write((byte)0);
                writer.Write((byte)0);
                writer.Write((byte)1);
                writer.Write(id);
            });
        }
        private static byte[] AddCheckBox(uint parent, uint id, string text)
        {
            return Write(delegate(BinaryWriter writer)
            {
                writer.Write((byte)0);
                writer.Write((byte)1);
                writer.Write(parent);
                writer.Write((byte)4);
                writer.Write(id);
                String(writer, text);
                writer.Write((byte)0);
            });
        }
        private static byte[] SetChecked(uint id, bool value)
        {
            return Write(delegate(BinaryWriter writer)
            {
                writer.Write((byte)5);
                writer.Write(id);
                writer.Write((byte)(value ? 1 : 0));
            });
        }
        private static byte[] AddLog(uint id, uint maxRows)
        {
            return Write(delegate(BinaryWriter writer)
            {
                writer.Write((byte)0);
                writer.Write((byte)0);
                writer.Write((byte)8);
                writer.Write(id);
                writer.Write(maxRows);
            });
        }
        private static byte[] AddComboBox(uint id)
        {
            return Write(delegate(BinaryWriter writer)
            {
                writer.Write((byte)0);
                writer.Write((byte)0);
                writer.Write((byte)6);
                writer.Write(id);
                writer.Write((uint)1);
                String(writer, "one");
                writer.Write((byte)0);
            });
        }
        private static byte[] SetItems(uint id)
        {
            return Write(delegate(BinaryWriter writer)
            {
                writer.Write((byte)6);
                writer.Write(id);
                writer.Write((uint)2);
                String(writer, "one");
                String(writer, "two");
                writer.Write((byte)1);
                writer.Write((uint)1);
            });
        }
        private static byte[] AddNumber(uint id)
        {
            return Write(delegate(BinaryWriter writer)
            {
                writer.Write((byte)0);
                writer.Write((byte)0);
                writer.Write((byte)7);
                writer.Write(id);
                writer.Write((long)0);
                writer.Write((long)10);
                writer.Write((long)1);
                writer.Write((long)5);
            });
        }
        private static byte[] SetNumber(uint id, long value)
        {
            return Write(delegate(BinaryWriter writer)
            {
                writer.Write((byte)8);
                writer.Write(id);
                writer.Write(value);
            });
        }
        private static byte[] AddLabel(uint parent, uint id)
        {
            return Write(delegate(BinaryWriter writer)
            {
                writer.Write((byte)0);
                writer.Write((byte)1);
                writer.Write(parent);
                writer.Write((byte)2);
                writer.Write(id);
                String(writer, "label");
            });
        }
        private static byte[] AppendLog(uint id, string line)
        {
            return Write(delegate(BinaryWriter writer)
            {
                writer.Write((byte)9);
                writer.Write(id);
                String(writer, line);
            });
        }
        private static byte[] Remove(uint id)
        {
            return Write(delegate(BinaryWriter writer)
            {
                writer.Write((byte)1);
                writer.Write(id);
            });
        }
        private static byte[] Write(Action<BinaryWriter> action)
        {
            using (MemoryStream stream = new MemoryStream())
            using (BinaryWriter writer = new BinaryWriter(stream))
            {
                action(writer);
                return stream.ToArray();
            }
        }
        private static void String(BinaryWriter writer, string value)
        {
            byte[] bytes = Encoding.UTF8.GetBytes(value);
            writer.Write((uint)bytes.Length);
            writer.Write(bytes);
        }
    }
}
