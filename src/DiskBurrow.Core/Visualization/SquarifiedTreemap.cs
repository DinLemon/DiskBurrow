namespace DiskBurrow.Core.Visualization;

public readonly record struct MapRect(double X,double Y,double Width,double Height)
{
 public double Area=>Width*Height;
 public bool Contains(double x,double y)=>x>=X&&y>=Y&&x<X+Width&&y<Y+Height;
}
public readonly record struct MapWeight(int Index,double Weight);
public readonly record struct MapTile(int Index,MapRect Bounds);
/// <summary>Squarified treemap, Bruls/Huizing/van Wijk. Ideas reviewed against MIT disktree (Tobi Lutke, 2026).</summary>
public static class SquarifiedTreemap
{
 public static IReadOnlyList<MapTile> Layout(IEnumerable<MapWeight> weights,MapRect bounds)
 {
  if(!double.IsFinite(bounds.Area)||bounds.Width<=0||bounds.Height<=0)return [];
  var items=weights.Where(x=>double.IsFinite(x.Weight)&&x.Weight>0).OrderByDescending(x=>x.Weight).ThenBy(x=>x.Index).ToArray();
  if(items.Length==0)return [];
  // Scaling by the largest weight avoids overflow for very large totals.
  var max=items[0].Weight; var total=items.Sum(x=>x.Weight/max);
  var areas=items.Select(x=>new MapWeight(x.Index,(x.Weight/max)/total*bounds.Area)).ToArray();
  var result=new List<MapTile>(areas.Length);var remaining=bounds;var start=0;
  while(start<areas.Length)
  {
   var side=Math.Min(remaining.Width,remaining.Height);var end=start+1;var sum=areas[start].Weight;
   var worst=Worst(areas[start].Weight,areas[start].Weight,sum,side);
   while(end<areas.Length)
   {
    var candidate=sum+areas[end].Weight;var next=Worst(areas[start].Weight,areas[end].Weight,candidate,side);
    if(next>worst)break;sum=candidate;worst=next;end++;
   }
   if(remaining.Width>=remaining.Height)
   {
    var width=end==areas.Length?remaining.Width:sum/remaining.Height;var y=remaining.Y;
    for(var i=start;i<end;i++){var height=i==end-1?remaining.Y+remaining.Height-y:areas[i].Weight/width;result.Add(new(areas[i].Index,new(remaining.X,y,width,Math.Max(0,height))));y+=height;}
    remaining=new(remaining.X+width,remaining.Y,Math.Max(0,remaining.Width-width),remaining.Height);
   }
   else
   {
    var height=end==areas.Length?remaining.Height:sum/remaining.Width;var x=remaining.X;
    for(var i=start;i<end;i++){var width=i==end-1?remaining.X+remaining.Width-x:areas[i].Weight/height;result.Add(new(areas[i].Index,new(x,remaining.Y,Math.Max(0,width),height)));x+=width;}
    remaining=new(remaining.X,remaining.Y+height,remaining.Width,Math.Max(0,remaining.Height-height));
   }
   start=end;
  }
  return result;
 }
 private static double Worst(double max,double min,double sum,double side)=>side<=0||min<=0?double.PositiveInfinity:Math.Max(side*side*max/(sum*sum),sum*sum/(side*side*min));
}
