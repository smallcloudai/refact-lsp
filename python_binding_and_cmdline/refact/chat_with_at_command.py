# chat_with_at_command.py
import sys

def chat():
    print("Chat started. Type 'exit' to quit.")
    while True:
        user_input = input("You: ")
        if user_input.lower() == 'exit':
            print("Chat ended.")
            break
        # Add your chat logic here
        response = generate_response(user_input)  # Call function to generate a response
        print(f"Bot: {response}")

def generate_response(user_input):
    # Simple echo response logic; you can customize this for your application
    # Here, you can add more sophisticated logic or connect to a chatbot API
    return f"Echo: {user_input}"

if __name__ == "__main__":
    chat()
