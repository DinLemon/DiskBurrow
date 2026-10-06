using DiskBurrow.Core.Visualization;
namespace DiskBurrow.Tests;
public sealed class TreemapLayoutTests
{
 [Theory] [InlineData(1000,600)] [InlineData(50,1000)] [InlineData(1000,50)]
 public void Layout_preserves_area_proportion_containment_and_nonoverlap(double width,double height)
 {
  var weights=new[]{100d,20,3,1,.001};var tiles=SquarifiedTreemap.Layout(weights.Select((v,i)=>new MapWeight(i,v)),new(10,20,width,height));
  Assert.Equal(weights.Length,tiles.Count);Assert.Equal(width*height,tiles.Sum(x=>x.Bounds.Area),6);
  foreach(var tile in tiles){Assert.InRange(tile.Bounds.X,10,10+width);Assert.InRange(tile.Bounds.Y,20,20+height);Assert.True(tile.Bounds.X+tile.Bounds.Width<=10+width+.00001);Assert.True(tile.Bounds.Y+tile.Bounds.Height<=20+height+.00001);Assert.Equal(weights[tile.Index]/weights.Sum()*width*height,tile.Bounds.Area,5);}
  foreach(var a in tiles)foreach(var b in tiles.Where(x=>x.Index!=a.Index))Assert.True(Math.Min(a.Bounds.X+a.Bounds.Width,b.Bounds.X+b.Bounds.Width)-Math.Max(a.Bounds.X,b.Bounds.X)<=.00001||Math.Min(a.Bounds.Y+a.Bounds.Height,b.Bounds.Y+b.Bounds.Height)-Math.Max(a.Bounds.Y,b.Bounds.Y)<=.00001);
 }
 [Fact] public void Zero_empty_invalid_and_extreme_weights_are_stable()
 {
  Assert.Empty(SquarifiedTreemap.Layout([new(0,0),new(1,double.NaN)],new(0,0,100,100)));
  var result=SquarifiedTreemap.Layout([new(0,1e300),new(1,1e299)],new(0,0,100,100));Assert.Equal(2,result.Count);Assert.Equal(10000,result.Sum(x=>x.Bounds.Area),6);
 }
}
