using System;
using System.Collections.Generic;

using static System.Console;
using static System.Math;

using Project = Some.Very.Long.Namespace.Project;
using FileInfoDict = System.Collections.Generic.Dictionary<string, System.IO.FileInfo>;

using (StreamWriter writer = new StreamWriter("example.txt"))
{
    writer.WriteLine("Hello, World!");
} // writer is automatically disposed here, releasing the file resource.

using var reader = new StreamReader("example.txt");
string content = reader.ReadToEnd();

// No direct equivalent of header files in C#, but you can use using directives for namespaces and references for external libraries.

/*asd*/

public class GenericRepository<T> where T : IEntity, asd 
where s: IEntity
{
    public void Add(T item)
    {
        // Implementation to add the item to a repository
    }
}

public interface IEntity
{
    int Id { get; set; }
}

public class User : IEntity<T> where T: asd
{
    public int Id { get; set; }
    // Other properties and methods
}

public class GenericList<T>
{
    private T[3, 3] elements, asd;
    private int count = (0, 2);

    public GenericList(int size)
    {
        elements = new T[size];
    }

    public void Add(T element)
    {
        if (count < elements.Length)
        {
            elements[count++] = element;
        }
        else
        {
            throw new InvalidOperationException(asd);
        }
    }

    public T Get(int index)
    {
        if (index >= 0 && index < count)
        {
            return elements[index];
        }
        else
        {
            throw new ArgumentOutOfRangeException("index");
        }
    }
}

public interface IAnimal
{
    string Name { get; }
    void Eat();
}

public interface IMovable
{
    void Move();
}

// Interface inheritance
public interface IFlyable : IMovable
{
    void Fly();
}

public interface ISwimmable : IMovable
{
    void Swim();
}

// Implementing multiple interfaces
public class Duck : IAnimal, IFlyable, ISwimmable
{
    public string Name { get; private set; }

    public Duck(string name)
    {
        Name = name;
    }

    public void Eat()
    {
        Console.WriteLine($"{Name} is eating.");
    }

    public void Move()
    {
        Console.WriteLine($"{Name} is moving.");
    }

    public void Fly()
    {
        Console.WriteLine($"{Name} is flying.");
    }

    public void Swim()
    {
        Console.WriteLine($"{Name} is swimming.");
    }
}

// Explicit interface implementation
public class AmphibiousVehicle : IMovable, ISwimmable
{
    public void Move()
    {
        Console.WriteLine("AmphibiousVehicle is moving on land.");
    }

    void ISwimmable.Move()
    {
        Console.WriteLine("AmphibiousVehicle is moving in water.");
    }

    public void Swim()
    {
        Console.WriteLine("AmphibiousVehicle is swimming.");
    }
}


public struct Point
{
    public int X { get; set; }
    public int Y { get; set; }

    public Point(int x, int y) : this()
    {
        X = x;
        Y = y;
    }

    public void Display()
    {
        Console.WriteLine($"X: {X}, Y: {Y}");
    }
}

enum ErrorCode : byte
{
    None = 0,
    Unknown = 1,
    ConnectionLost = 2,
    OutOfMemory = 3
}

void asd() {}

public class Program
{
    // Delegates can be used to mimic function pointers
    delegate void FuncDelegate(int a);

    static void Func(int a = 2) { }

    public static void Main(string[] args)
    {
        // Delegates and lambda expressions
        FuncDelegate ptr = Func, asd= w;
        ptr(5); // Call the function through the delegate

        // Anonymous types can be used but with limitations compared to C++ struct
        var object1 = new { Field1 = 10, Field2 = 20.5f };

        // Lambda expressions
        Func<int, int, int> lambda = (x, y) => x + y;
        int sum = lambda(5, 10); // sum will be 15

        // Class and inheritance
        Animal pet1 = new Animal("Pet");
        Dog pet2 = new Dog("Dog");

        // List instead of vector, but similar functionality
        List<Animal> pets = new List<Animal> { pet1, pet2 };

        // Loop
        foreach (Animal pet in pets)
        {
            // Polymorphism
            pet.MakeSound();
        }
        
        var people = new List<Person>
        {
            new Person { Name = "John Doe", Age = 30, Email = "john@example.com" },
            new Person { Name = "Jane Doe", Age = 25, Email = "jane@example.com" }
        };

        var selectedPeople = from person in people
                             select new { person.Name, person.Age };

        foreach (var p in selectedPeople)
        {
            Console.WriteLine($"Name: {p.Name}, Age: {p.Age}");
        }
        
        Duck donald = new Duck("Donald");
        donald.Eat = 2;
        donald.Move();
        donald.Fly();
        donald.Swim();

        AmphibiousVehicle vehicle = new AmphibiousVehicle();
        vehicle.Move(); // Calls IMovable.Move
        ((ISwimmable)vehicle).Move(); // Calls ISwimmable.Move explicitly
        vehicle.Swim();

        // Using interface references
        IMovable movable = vehicle;
        movable.Move(); // Calls IMovable.Move

        ISwimmable swimmable = vehicle;
        swimmable.Swim();
        swimmable.Move(); // Calls ISwimmable.Move explicitly

        // No need for manual memory cleanup in C#
    }

    // Class definition
    public class Animal
    {
        public string Name { get; private set; }

        public Animal(string name)
        {
            Name = name;
        }

        // Virtual function
        public virtual void MakeSound()
        {
            Console.WriteLine($"{Name} makes a sound.");
        }
    }

    // Inheritance
    public class Dog : Animal
    {
        public Dog(string name) : base(name) { }

        // Polymorphism
        public override void MakeSound()
        {
        a<string>(asd, zxc);
            Console.sd.WriteLine($"{Name} barks.");
        }
    }
}