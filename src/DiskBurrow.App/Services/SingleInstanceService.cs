using System.IO.Pipes;
using System.Security.Cryptography;
using System.Security.Principal;
using System.Text;
namespace DiskBurrow.App.Services;
public sealed class SingleInstanceService : IAsyncDisposable
{
    private readonly string name;
    private readonly Action activated;
    private readonly CancellationTokenSource stop=new();
    private Mutex? mutex;
    private Task? listener;
    public SingleInstanceService(string applicationName,Action activated)
    {
        var sid=WindowsIdentity.GetCurrent().User?.Value ?? throw new InvalidOperationException("Current user SID unavailable.");
        name=applicationName+"-"+Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(sid)))[..24];
        this.activated=activated;
    }
    public async Task<bool> TryBecomePrimaryAsync()
    {
        mutex=new Mutex(false,@"Global\"+name,out var created);
        // Named-object lifetime, rather than thread-affine ownership, protects asynchronous shutdown.
        if(created){listener=ListenAsync();return true;}
        using var client=new NamedPipeClientStream(".",name,PipeDirection.Out,PipeOptions.Asynchronous|PipeOptions.CurrentUserOnly);
        using var timeout=new CancellationTokenSource(TimeSpan.FromSeconds(5));
        await client.ConnectAsync(timeout.Token);await client.WriteAsync(new byte[]{1},timeout.Token);return false;
    }
    private async Task ListenAsync()
    {
        while(!stop.IsCancellationRequested)
        {
            try {
                await using var server=new NamedPipeServerStream(name,PipeDirection.In,1,PipeTransmissionMode.Byte,PipeOptions.Asynchronous|PipeOptions.CurrentUserOnly);
                await server.WaitForConnectionAsync(stop.Token).ConfigureAwait(false);
                var buffer=new byte[1];if(await server.ReadAsync(buffer,stop.Token).ConfigureAwait(false)==1 && buffer[0]==1)activated();
            }catch(OperationCanceledException)when(stop.IsCancellationRequested){break;}
            catch(IOException) { }
        }
    }
    public async ValueTask DisposeAsync(){stop.Cancel();if(listener is not null)await listener.ConfigureAwait(false);mutex?.Dispose();stop.Dispose();}
}
