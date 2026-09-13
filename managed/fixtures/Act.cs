using System.Reflection;
using System.Windows.Forms;

[assembly: AssemblyVersion("1.2.3.4")]

namespace Advanced_Combat_Tracker
{
    public interface IActPluginV1
    {
        void InitPlugin(TabPage page, Label status);
        void DeInitPlugin();
    }
}
